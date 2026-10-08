//! Against the Python `wormhole` CLI and mailbox server. Needs `wormhole` and
//! `uvx` on PATH: `cargo test -p wyrmyon --test interop -- --ignored`.

mod support;

use std::net::TcpListener;
use std::process::Stdio;
use std::time::Duration;

use support::{finish, read_code, wyrm};
use tokio::process::{Child, Command};

struct PythonMailbox {
    child: Child,
    url: String,
    _dir: tempfile::TempDir,
}

impl PythonMailbox {
    async fn start() -> Self {
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let dir = tempfile::tempdir().unwrap();
        let child = Command::new("uvx")
            .args([
                "--from",
                "magic-wormhole-mailbox-server",
                "twist",
                "wormhole-mailbox",
            ])
            .arg(format!("--port=tcp:{port}:interface=127.0.0.1"))
            .arg(format!(
                "--channel-db={}",
                dir.path().join("relay.sqlite").display()
            ))
            .current_dir(dir.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .kill_on_drop(true)
            .spawn()
            .expect("uvx on PATH");
        for _ in 0..300 {
            if tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_ok()
            {
                return Self {
                    child,
                    url: format!("ws://127.0.0.1:{port}/v1"),
                    _dir: dir,
                };
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("the Python mailbox server did not start");
    }
}

impl Drop for PythonMailbox {
    fn drop(&mut self) {
        if let Some(group) = self.child.id() {
            let _ = std::process::Command::new("kill")
                .args(["-TERM", "--", &format!("-{group}")])
                .status();
        }
    }
}

fn python(relay: &str) -> Command {
    let mut cmd = Command::new("wormhole");
    cmd.args(["--relay-url", relay])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    cmd
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the Python wormhole CLI and uvx"]
async fn text_from_python_to_wyrm() {
    let mailbox = PythonMailbox::start().await;
    let mut sender = python(&mailbox.url)
        .args(["send", "--hide-progress", "--text", "from python"])
        .spawn()
        .unwrap();
    let code = read_code(sender.stderr.take().unwrap()).await;
    let (ok, stdout, stderr) =
        finish(wyrm(&mailbox.url).args(["receive", &code]).spawn().unwrap()).await;
    assert!(ok, "{stderr}");
    assert_eq!(stdout, "from python\n");
    assert!(finish(sender).await.0);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the Python wormhole CLI and uvx"]
async fn text_from_wyrm_to_python() {
    let mailbox = PythonMailbox::start().await;
    let mut sender = wyrm(&mailbox.url)
        .args(["send", "--text", "from wyrm"])
        .spawn()
        .unwrap();
    let code = read_code(sender.stderr.take().unwrap()).await;
    let (ok, stdout, stderr) = finish(
        python(&mailbox.url)
            .args(["receive", "--hide-progress", &code])
            .spawn()
            .unwrap(),
    )
    .await;
    assert!(ok, "{stderr}");
    assert!(stdout.contains("from wyrm"), "{stdout}");
    assert!(finish(sender).await.0);
}
