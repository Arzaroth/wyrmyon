use selvedge::update::Owner;
use selvedge::{Project, Target};

#[cfg(windows)]
const BINARIES: &[&str] = &["wyrmyon.exe", "wyrm.exe"];
#[cfg(not(windows))]
const BINARIES: &[&str] = &["wyrmyon"];

#[cfg(windows)]
const ALIASES: &[&str] = &[];
#[cfg(not(windows))]
const ALIASES: &[&str] = &["wyrm"];

const TARGETS: &[Target] = &[
    Target {
        os: "linux",
        arch: "x86_64",
        name: "x86_64-unknown-linux-musl",
    },
    Target {
        os: "linux",
        arch: "aarch64",
        name: "aarch64-unknown-linux-musl",
    },
    Target {
        os: "macos",
        arch: "x86_64",
        name: "x86_64-apple-darwin",
    },
    Target {
        os: "macos",
        arch: "aarch64",
        name: "aarch64-apple-darwin",
    },
    Target {
        os: "windows",
        arch: "x86_64",
        name: "x86_64-pc-windows-msvc",
    },
    Target {
        os: "windows",
        arch: "aarch64",
        name: "aarch64-pc-windows-msvc",
    },
];

const WYRMYON: Project = Project {
    binaries: BINARIES,
    repo: "Arzaroth/wyrmyon",
    repo_env: "WYRMYON_REPO",
    version: env!("CARGO_PKG_VERSION"),
    frontends: &[],
    aliases: ALIASES,
    legacy: &[],
    targets: TARGETS,
    msi_marker_key: Some(r"HKCU\Software\wyrmyon"),
};

pub(crate) fn run(hide_progress: bool) -> anyhow::Result<()> {
    refuse_if_owned(selvedge::update::owner(&WYRMYON)?)?;
    if hide_progress {
        selvedge::update::silence_progress();
    }
    let (xdg, home) = crate::cache_env();
    let cache = crate::cache_root_from(xdg, home)?.join("update.json");
    let _ = rustls::crypto::ring::default_provider().install_default();
    let applied = selvedge::update::apply(&WYRMYON, &cache)?;
    println!("{}", outcome(WYRMYON.version, &applied.version));
    Ok(())
}

fn refuse_if_owned(owner: Option<Owner>) -> anyhow::Result<()> {
    match owner {
        Some(owner) => anyhow::bail!("{}", owner.advice(&WYRMYON)),
        None => Ok(()),
    }
}

fn outcome(running: &str, installed: &str) -> String {
    if running == installed {
        format!("wyrmyon {running} is the latest release")
    } else {
        format!("wyrmyon updated from {running} to {installed}")
    }
}

