use std::process::Command;

#[test]
fn the_binary_writes_the_assets_or_explains_its_usage() {
    let dir = tempfile::tempdir().unwrap();
    let ok = Command::new(env!("CARGO_BIN_EXE_wyrmyon-xtask"))
        .arg("assets")
        .arg(dir.path())
        .status()
        .unwrap();
    assert!(ok.success());
    assert!(dir.path().join("completions/_wyrm").exists());

    let usage = Command::new(env!("CARGO_BIN_EXE_wyrmyon-xtask"))
        .output()
        .unwrap();
    assert!(!usage.status.success());
    assert!(String::from_utf8_lossy(&usage.stderr).contains("usage:"));
}
