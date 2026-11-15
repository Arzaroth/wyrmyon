mod support;

use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use support::{TIMEOUT, wyrm};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::Child;
use tokio::sync::mpsc;
use wyrmyon_testkit::MailboxServer;
use wyrmyon_transport_iroh::{IrohTransport, Relays, Role as IrohRole};
use wyrmyon_wormhole::{Config, Mood, Wormhole};

struct Wyrm {
    child: Child,
    lines: mpsc::UnboundedReceiver<String>,
    seen: String,
}

impl Wyrm {
    fn start(server: &MailboxServer, args: &[&str]) -> Self {
        Self::spawn(wyrm(&server.url()).arg("forward").args(args))
    }

    fn typing(server: &MailboxServer, code: &str) -> Self {
        let mut wyrm = Self::spawn(wyrm(&server.url()).arg("forward").stdin(Stdio::piped()));
        let mut stdin = wyrm.child.stdin.take().unwrap();
        let line = format!("{code}\n");
        tokio::spawn(async move { stdin.write_all(line.as_bytes()).await });
        wyrm
    }

    fn spawn(command: &mut tokio::process::Command) -> Self {
        let mut child = command.spawn().unwrap();
        let stderr = child.stderr.take().unwrap();
        let (tell, lines) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut stderr = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = stderr.next_line().await {
                let _ = tell.send(line);
            }
        });
        Self {
            child,
            lines,
            seen: String::new(),
        }
    }

    async fn line_with(&mut self, needle: &str) -> String {
        loop {
            let line = tokio::time::timeout(TIMEOUT, self.lines.recv())
                .await
                .unwrap_or_else(|_| panic!("no line with {needle:?} in:\n{}", self.seen))
                .unwrap_or_else(|| panic!("exited before {needle:?}:\n{}", self.seen));
            self.seen.push_str(&line);
            self.seen.push('\n');
            if line.contains(needle) {
                return line;
            }
        }
    }

    async fn code(&mut self) -> String {
        let line = self.line_with("code is: ").await;
        line.split("code is: ").nth(1).unwrap().trim().to_owned()
    }

    async fn listening(&mut self) -> u16 {
        let line = self.line_with("Listening on ").await;
        let addr = line
            .strip_prefix("Listening on ")
            .unwrap()
            .split(' ')
            .next()
            .unwrap();
        addr.rsplit(':').next().unwrap().parse().unwrap()
    }

    async fn end(mut self) -> (bool, String) {
        let status = tokio::time::timeout(TIMEOUT, self.child.wait())
            .await
            .expect("wyrm forward never exited")
            .unwrap();
        while let Some(line) = self.lines.recv().await {
            self.seen.push_str(&line);
            self.seen.push('\n');
        }
        (status.success(), self.seen)
    }

    #[cfg(unix)]
    fn interrupt(&self) {
        let pid = self.child.id().unwrap().to_string();
        let killed = std::process::Command::new("kill")
            .args(["-INT", &pid])
            .status()
            .unwrap();
        assert!(killed.success());
    }
}

async fn echo() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let (mut tcp, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut got = Vec::new();
                tcp.read_to_end(&mut got).await.unwrap();
                got.reverse();
                tcp.write_all(&got).await.unwrap();
            });
        }
    });
    port
}

async fn round_trip(port: u16, data: &[u8]) -> Vec<u8> {
    let mut tcp = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    tcp.write_all(data).await.unwrap();
    tcp.shutdown().await.unwrap();
    let mut back = Vec::new();
    let _ = tokio::time::timeout(TIMEOUT, tcp.read_to_end(&mut back)).await;
    back
}

async fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    listener.local_addr().unwrap().port()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shared_port_answers_on_the_other_side_until_it_stops() {
    let server = MailboxServer::start().await;
    let target = echo().await;
    let dead = free_port().await;
    let mut sharing = Wyrm::start(
        &server,
        &[&target.to_string(), &format!("127.0.0.1:{dead}")],
    );
    let code = sharing.code().await;
    let mut taking = Wyrm::start(
        &server,
        &[&code, &format!("0:{target}"), &format!("0:{dead}")],
    );
    let here = taking.listening().await;
    let dead_here = taking.listening().await;
    taking.line_with("Forwarding (iroh").await;
    sharing
        .line_with(&format!("Sharing localhost:{target} as port {target}"))
        .await;
    sharing.line_with("Forwarding (iroh").await;

    assert_eq!(round_trip(here, b"stressed").await, b"desserts");
    sharing
        .line_with(&format!("Connection to localhost:{target}"))
        .await;
    assert_eq!(round_trip(dead_here, b"x").await, b"");
    sharing
        .line_with(&format!("Could not reach 127.0.0.1:{dead}"))
        .await;
    taking
        .line_with(&format!("Their side could not reach its port {dead}"))
        .await;

    stop(taking, sharing).await;
}

#[cfg(unix)]
async fn stop(taking: Wyrm, sharing: Wyrm) {
    taking.interrupt();
    let (ok, said) = taking.end().await;
    assert!(ok, "{said}");
    assert!(said.contains("Stopped forwarding"), "{said}");
    let (ok, said) = sharing.end().await;
    assert!(ok, "{said}");
    assert!(said.contains("The other side stopped forwarding"), "{said}");
}

#[cfg(not(unix))]
async fn stop(mut taking: Wyrm, mut sharing: Wyrm) {
    taking.child.kill().await.unwrap();
    sharing.child.kill().await.unwrap();
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn the_sharing_side_can_stop_it_too_and_ctrl_c_before_pairing_fails() {
    let server = MailboxServer::start().await;
    let target = echo().await;
    let mut sharing = Wyrm::start(
        &server,
        &["--code", "8-guitarist-revenge", &target.to_string()],
    );
    sharing.code().await;
    let mut taking = Wyrm::start(&server, &["8-guitarist-revenge", &format!("0:{target}")]);
    let here = taking.listening().await;
    sharing.line_with("Forwarding (iroh").await;
    assert_eq!(round_trip(here, b"ab").await, b"ba");
    sharing.interrupt();
    let (ok, said) = taking.end().await;
    assert!(ok, "{said}");
    assert!(said.contains("The other side stopped forwarding"), "{said}");
    let (ok, said) = sharing.end().await;
    assert!(ok && said.contains("Stopped forwarding"), "{said}");

    let mut alone = Wyrm::start(&server, &["22"]);
    alone.code().await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    alone.interrupt();
    let (ok, said) = alone.end().await;
    assert!(!ok && said.contains("interrupted"), "{said}");
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn without_maps_every_shared_port_listens_under_its_own_number() {
    let server = MailboxServer::start().await;
    let target = echo().await;
    let mut sharing = Wyrm::start(&server, &[&format!("127.0.0.1:{target}"), "--qr"]);
    let code = sharing.code().await;
    let mut taking = Wyrm::start(&server, &["--bind", "127.0.0.2", "--code", &code]);
    assert_eq!(taking.listening().await, target);
    taking.line_with("Forwarding (iroh").await;
    let mut tcp = TcpStream::connect(("127.0.0.2", target)).await.unwrap();
    tcp.write_all(b"level").await.unwrap();
    tcp.shutdown().await.unwrap();
    let mut back = Vec::new();
    tcp.read_to_end(&mut back).await.unwrap();
    assert_eq!(back, b"level");
    stop(taking, sharing).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_port_the_other_side_does_not_share_fails_both_sides() {
    let server = MailboxServer::start().await;
    let mut sharing = Wyrm::start(&server, &["22"]);
    let code = sharing.code().await;
    let taking = Wyrm::start(&server, &[&code, "80"]);
    let (ok, said) = taking.end().await;
    assert!(!ok, "{said}");
    assert!(
        said.contains("does not share port 80 (it shares 22)"),
        "{said}"
    );
    let (ok, said) = sharing.end().await;
    assert!(!ok, "{said}");
    assert!(said.contains("cannot take this forward"), "{said}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_busy_local_port_fails_before_accepting() {
    let server = MailboxServer::start().await;
    let busy = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = busy.local_addr().unwrap().port();
    let mut sharing = Wyrm::start(&server, &[&port.to_string()]);
    let code = sharing.code().await;
    let taking = Wyrm::typing(&server, &code);
    let (ok, said) = taking.end().await;
    assert!(
        !ok && said.contains(&format!("listening on 127.0.0.1:{port}")),
        "{said}"
    );
    let (ok, _) = sharing.end().await;
    assert!(!ok);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plain_receiver_or_sender_is_turned_away() {
    let server = MailboxServer::start().await;
    let mut sharing = Wyrm::start(&server, &["22"]);
    let code = sharing.code().await;
    let receiver = wyrm(&server.url())
        .args(["receive", &code])
        .spawn()
        .unwrap();
    let (ok, said) = sharing.end().await;
    assert!(!ok && said.contains("not running `wyrm forward`"), "{said}");
    let (ok, _, said) = support::finish(receiver).await;
    assert!(
        !ok && said.contains("take it with `wyrm forward CODE`"),
        "{said}"
    );

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), b"a").unwrap();
    let mut sender = wyrm(&server.url())
        .args(["send", "a.txt"])
        .current_dir(dir.path())
        .spawn()
        .unwrap();
    let code = support::read_code(sender.stderr.take().unwrap()).await;
    let taking = Wyrm::start(&server, &[&code]);
    let (ok, said) = taking.end().await;
    assert!(!ok && said.contains("waiting for a port forward"), "{said}");
    let (ok, _, _) = support::finish(sender).await;
    assert!(!ok);
}

#[tokio::test(flavor = "multi_thread")]
async fn forward_refuses_what_cannot_work() {
    let server = MailboxServer::start().await;
    for (args, why) in [
        (&["--force-classic", "22"][..], "--force-classic"),
        (&["0"], "not a port number"),
        (&["22", "22"], "shared twice"),
        (&["--code", "3-a-b", "3-a-b"], "give the code once"),
    ] {
        let (ok, said) = Wyrm::start(&server, args).end().await;
        assert!(!ok && said.contains(why), "{args:?}: {said}");
    }
}

fn forward_config(server: &MailboxServer) -> Config {
    Config {
        relay_url: server.url(),
        app_versions: json!({"wyrmyon": {"transports": ["iroh-v1"], "offers": ["forward-v1"]}}),
        ..Config::default()
    }
}

async fn until(wormhole: &mut Wormhole, key: &str) -> Value {
    loop {
        let msg = tokio::time::timeout(TIMEOUT, wormhole.receive_json())
            .await
            .unwrap_or_else(|_| panic!("wyrm never sent {key}"))
            .unwrap();
        if let Some(value) = msg.get(key) {
            return value.clone();
        }
    }
}

async fn taken_by_hand(server: &MailboxServer, args: &[&str]) -> (Wormhole, Wyrm) {
    let pending = wyrmyon_wormhole::create(&forward_config(server), 2)
        .await
        .unwrap();
    let mut all = vec![pending.code().as_str()];
    all.extend_from_slice(args);
    let taking = Wyrm::start(server, &all);
    let wormhole = tokio::time::timeout(TIMEOUT, pending.pair())
        .await
        .unwrap()
        .unwrap();
    (wormhole, taking)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_taking_side_refuses_a_forward_it_cannot_use() {
    let server = MailboxServer::start().await;
    let (mut peer, taking) = taken_by_hand(&server, &[]).await;
    peer.send_json(&json!({"chat": 1})).await.unwrap();
    peer.send_json(&json!({"offer": {"wyrmyon-forward-v1": {"ports": [22]}}}))
        .await
        .unwrap();
    assert_eq!(
        until(&mut peer, "error").await,
        "a forward needs an iroh address"
    );
    let (ok, said) = taking.end().await;
    assert!(
        !ok && said.contains("a forward needs an iroh address"),
        "{said}"
    );
    peer.close(Mood::Happy).await;

    let (mut peer, taking) = taken_by_hand(&server, &[]).await;
    let iroh = IrohTransport::bind(IrohRole::Sender, Relays::Disabled)
        .await
        .unwrap();
    let info = serde_json::to_value(iroh.info().await).unwrap();
    peer.send_json(&json!({"wyrmyon-iroh-v1": info}))
        .await
        .unwrap();
    peer.send_json(&json!({"offer": {"wyrmyon-forward-v1": {"ports": []}}}))
        .await
        .unwrap();
    until(&mut peer, "wyrmyon-iroh-v1").await;
    assert_eq!(
        until(&mut peer, "error").await,
        "the other side cannot take this forward"
    );
    let (ok, said) = taking.end().await;
    assert!(!ok && said.contains("shares no port"), "{said}");
    iroh.close().await;
    peer.close(Mood::Happy).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_sharing_side_wants_a_forward_ack() {
    let server = MailboxServer::start().await;
    let mut sharing = Wyrm::start(&server, &["22"]);
    let code = sharing.code().await.parse().unwrap();
    let mut peer = wyrmyon_wormhole::join(&forward_config(&server), code)
        .await
        .unwrap()
        .pair()
        .await
        .unwrap();
    until(&mut peer, "wyrmyon-iroh-v1").await;
    assert_eq!(
        until(&mut peer, "offer").await,
        json!({"wyrmyon-forward-v1": {"ports": [22]}})
    );
    peer.send_json(&json!({"chat": 1})).await.unwrap();
    peer.send_json(&json!({"answer": {"file_ack": "ok"}}))
        .await
        .unwrap();
    let (ok, said) = sharing.end().await;
    assert!(!ok && said.contains("unexpected answer"), "{said}");
    peer.close(Mood::Happy).await;
}
