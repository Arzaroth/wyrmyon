#![allow(dead_code)]

use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::{Child, Command};

pub const TIMEOUT: Duration = Duration::from_secs(30);

pub fn wyrm(relay: &str) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_wyrm"));
    cmd.env("WYRMYON_RELAY_URL", relay)
        .env("WYRMYON_TRANSIT_HELPER", "tcp:127.0.0.1:9")
        .env("WYRMYON_IROH_RELAYS", "disabled")
        .env("WYRMYON_CACHE_DIR", cache_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    cmd
}

pub async fn read_code(stream: impl AsyncRead + Unpin + Send + 'static) -> String {
    let (found, code) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let mut found = Some(found);
        let mut lines = BufReader::new(stream).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(code) = line.split("code is: ").nth(1)
                && let Some(found) = found.take()
            {
                let _ = found.send(code.trim().to_owned());
            }
        }
    });
    tokio::time::timeout(TIMEOUT, code)
        .await
        .expect("the sender never printed a code")
        .expect("the sender exited without printing a code")
}

pub async fn finish(child: Child) -> (bool, String, String) {
    let out = tokio::time::timeout(TIMEOUT, child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

pub fn cache_dir() -> std::path::PathBuf {
    tempfile::Builder::new()
        .prefix("wyrmyon-test-cache")
        .tempdir()
        .unwrap()
        .keep()
}
