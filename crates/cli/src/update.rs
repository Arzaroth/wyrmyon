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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_package_managers_copy_is_left_to_the_package_manager() {
        let err = refuse_if_owned(Some(Owner::Distribution(Some("dpkg"))))
            .unwrap_err()
            .to_string();
        assert!(err.starts_with("wyrmyon was installed by"), "{err}");
        assert!(err.contains("(dpkg)"), "{err}");
        let err = refuse_if_owned(Some(Owner::Msi)).unwrap_err().to_string();
        assert!(
            err.contains("github.com/Arzaroth/wyrmyon/releases/latest"),
            "{err}"
        );
        assert!(refuse_if_owned(None).is_ok());
    }

    #[test]
    fn the_outcome_says_whether_anything_changed() {
        assert_eq!(
            outcome("0.2.0", "0.2.0"),
            "wyrmyon 0.2.0 is the latest release"
        );
        assert_eq!(
            outcome("0.2.0", "0.3.0"),
            "wyrmyon updated from 0.2.0 to 0.3.0"
        );
    }

    #[test]
    fn every_release_target_has_an_asset_name_the_updater_looks_for() {
        let workflow = include_str!("../../../.github/workflows/release.yml");
        let built: Vec<&str> = workflow
            .lines()
            .filter_map(|l| l.trim().strip_prefix("- target: "))
            .collect();
        assert_eq!(built.len(), TARGETS.len());
        for target in &built {
            assert!(
                TARGETS.iter().any(|t| t.name == *target),
                "{target} is released but the updater never asks for it"
            );
        }
        for t in TARGETS {
            assert!(built.contains(&t.name), "{} is never released", t.name);
            for other in TARGETS {
                assert!(
                    t == other || !other.name.contains(t.name),
                    "{} matches {}'s asset",
                    t.name,
                    other.name
                );
            }
        }
        assert!(selvedge::update::arch_target(&WYRMYON).is_ok());
    }

    #[test]
    fn the_archive_is_named_the_way_package_sh_names_it() {
        let script = include_str!("../../../scripts/package.sh");
        assert!(script.contains(r#"name="wyrmyon-v$version-$target""#));
        assert_eq!(WYRMYON.primary().trim_end_matches(".exe"), "wyrmyon");
    }

    #[test]
    fn the_msi_writes_the_marker_the_updater_reads() {
        let wxs = include_str!("../../../packaging/windows/wyrmyon.wxs");
        assert!(wxs.contains(r#"Key="Software\wyrmyon""#));
        assert!(wxs.contains(r#"Name="ProductCode""#));
        assert!(wxs.contains(r#"Value="[ProductCode]""#));
        assert_eq!(WYRMYON.msi_marker_key, Some(r"HKCU\Software\wyrmyon"));
    }
}
