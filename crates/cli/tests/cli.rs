use std::process::Command;

fn version_of(bin: &str) -> String {
    let out = Command::new(bin).arg("--version").output().unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn both_binaries_print_the_package_version() {
    let expected = format!("wyrmyon {}\n", env!("CARGO_PKG_VERSION"));
    assert_eq!(version_of(env!("CARGO_BIN_EXE_wyrmyon")), expected);
    assert_eq!(version_of(env!("CARGO_BIN_EXE_wyrm")), expected);
}
