//! Against the Python `wormhole` CLI and mailbox server. Needs `wormhole` and
//! `uvx` on PATH: `cargo test -p wyrmyon --test interop -- --ignored`.

mod support;

use std::net::TcpListener;
use std::process::Stdio;
use std::time::Duration;

use support::{finish, read_code, wyrm};
use tokio::process::{Child, Command};

struct PythonServer {
    child: Child,
    port: u16,
    _dir: tempfile::TempDir,
}

struct PythonMailbox {
    _server: PythonServer,
    url: String,
}

impl PythonMailbox {
    async fn start() -> Self {
        let server = PythonServer::start(
            "magic-wormhole-mailbox-server",
            "wormhole-mailbox",
            |port, dir| {
                vec![
                    format!("--port=tcp:{port}:interface=127.0.0.1"),
                    format!("--channel-db={}", dir.join("relay.sqlite").display()),
                ]
            },
        )
        .await;
        let url = format!("ws://127.0.0.1:{}/v1", server.port);
        Self {
            _server: server,
            url,
        }
    }
}

impl PythonServer {
    async fn start(
        package: &str,
        plugin: &str,
        args: impl FnOnce(u16, &std::path::Path) -> Vec<String>,
    ) -> Self {
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let dir = tempfile::tempdir().unwrap();
        let child = Command::new("uvx")
            .args(["--from", package, "twist", plugin])
            .args(args(port, dir.path()))
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
                    port,
                    _dir: dir,
                };
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("the Python {plugin} server did not start");
    }
}

impl Drop for PythonServer {
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
    cmd.args(["--relay-url", relay, "--transit-helper", "tcp:127.0.0.1:9"])
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

fn blob(len: usize) -> Vec<u8> {
    let mut state = 0x2545_f491_4f6c_dd1d_u64;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state.to_le_bytes()[0]
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the Python wormhole CLI and uvx"]
async fn file_from_wyrm_to_python() {
    let mailbox = PythonMailbox::start().await;
    let (from, to) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let data = blob(3_000_000);
    std::fs::write(from.path().join("data.bin"), &data).unwrap();
    let mut sender = wyrm(&mailbox.url)
        .args(["send", "data.bin"])
        .current_dir(from.path())
        .spawn()
        .unwrap();
    let code = read_code(sender.stderr.take().unwrap()).await;
    let (ok, _, stderr) = finish(
        python(&mailbox.url)
            .args(["receive", "--hide-progress", "--accept-file", &code])
            .current_dir(to.path())
            .spawn()
            .unwrap(),
    )
    .await;
    assert!(ok, "{stderr}");
    assert!(finish(sender).await.0);
    assert_eq!(std::fs::read(to.path().join("data.bin")).unwrap(), data);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the Python wormhole CLI and uvx"]
async fn file_from_python_to_wyrm() {
    let mailbox = PythonMailbox::start().await;
    let (from, to) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let data = blob(3_000_000);
    std::fs::write(from.path().join("data.bin"), &data).unwrap();
    let mut sender = python(&mailbox.url)
        .args(["send", "--hide-progress", "data.bin"])
        .current_dir(from.path())
        .spawn()
        .unwrap();
    let code = read_code(sender.stderr.take().unwrap()).await;
    let (ok, _, stderr) = finish(
        wyrm(&mailbox.url)
            .args(["receive", "--accept-file", &code])
            .current_dir(to.path())
            .spawn()
            .unwrap(),
    )
    .await;
    assert!(ok, "{stderr}");
    assert!(finish(sender).await.0);
    assert_eq!(std::fs::read(to.path().join("data.bin")).unwrap(), data);
}

async fn python_transit_relay() -> (PythonServer, String) {
    let server = PythonServer::start("magic-wormhole-transit-relay", "transitrelay", |port, _| {
        vec![format!("--port=tcp:{port}:interface=127.0.0.1")]
    })
    .await;
    let hint = format!("tcp:127.0.0.1:{}", server.port);
    (server, hint)
}

fn tree_in(dir: &std::path::Path) -> std::path::PathBuf {
    let tree = dir.join("tree");
    std::fs::create_dir_all(tree.join("sub/empty")).unwrap();
    std::fs::write(tree.join("a.txt"), b"alpha").unwrap();
    std::fs::write(tree.join("sub/b.bin"), blob(500_000)).unwrap();
    tree
}

fn assert_same_tree(got: &std::path::Path) {
    assert_eq!(std::fs::read(got.join("a.txt")).unwrap(), b"alpha");
    assert_eq!(std::fs::read(got.join("sub/b.bin")).unwrap(), blob(500_000));
    assert!(got.join("sub/empty").is_dir());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the Python wormhole CLI and uvx"]
async fn directory_from_wyrm_to_python_through_the_relay() {
    let mailbox = PythonMailbox::start().await;
    let (_relay, hint) = python_transit_relay().await;
    let (from, to) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    tree_in(from.path());
    let mut sender = wyrm(&mailbox.url)
        .args(["--no-listen", "--transit-helper", &hint, "send", "tree"])
        .current_dir(from.path())
        .spawn()
        .unwrap();
    let code = read_code(sender.stderr.take().unwrap()).await;
    let (ok, _, stderr) = finish(
        python(&mailbox.url)
            .args([
                "--transit-helper",
                &hint,
                "receive",
                "--no-listen",
                "--hide-progress",
                "--accept-file",
                &code,
            ])
            .current_dir(to.path())
            .spawn()
            .unwrap(),
    )
    .await;
    assert!(ok, "{stderr}");
    assert!(finish(sender).await.0);
    assert_same_tree(&to.path().join("tree"));
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the Python wormhole CLI and uvx"]
async fn directory_from_python_to_wyrm_through_the_relay() {
    let mailbox = PythonMailbox::start().await;
    let (_relay, hint) = python_transit_relay().await;
    let (from, to) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    tree_in(from.path());
    let mut sender = python(&mailbox.url)
        .args([
            "--transit-helper",
            &hint,
            "send",
            "--no-listen",
            "--hide-progress",
            "tree",
        ])
        .current_dir(from.path())
        .spawn()
        .unwrap();
    let code = read_code(sender.stderr.take().unwrap()).await;
    let (ok, _, stderr) = finish(
        wyrm(&mailbox.url)
            .args([
                "--no-listen",
                "--transit-helper",
                &hint,
                "receive",
                "--accept-file",
                &code,
            ])
            .current_dir(to.path())
            .spawn()
            .unwrap(),
    )
    .await;
    assert!(ok, "{stderr}");
    assert!(stderr.contains("via relay"), "{stderr}");
    assert!(finish(sender).await.0);
    assert_same_tree(&to.path().join("tree"));
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the Python wormhole CLI and uvx"]
async fn python_sends_to_a_code_wyrm_allocated() {
    let mailbox = PythonMailbox::start().await;
    let (from, to) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::write(from.path().join("r.txt"), b"receiver first").unwrap();
    let mut receiver = wyrm(&mailbox.url)
        .args(["receive", "--new", "--accept-file"])
        .current_dir(to.path())
        .spawn()
        .unwrap();
    let code = read_code(receiver.stderr.take().unwrap()).await;
    let (ok, _, stderr) = finish(
        python(&mailbox.url)
            .args(["send", "--hide-progress", "--code", &code, "r.txt"])
            .current_dir(from.path())
            .spawn()
            .unwrap(),
    )
    .await;
    assert!(ok, "{stderr}");
    assert!(finish(receiver).await.0);
    assert_eq!(
        std::fs::read(to.path().join("r.txt")).unwrap(),
        b"receiver first"
    );
}
