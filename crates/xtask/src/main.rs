use std::path::Path;

use anyhow::{Context, bail};
use clap_complete::Shell;

const BINARIES: [&str; 2] = ["wyrmyon", "wyrm"];
const SHELLS: [Shell; 5] = [
    Shell::Bash,
    Shell::Zsh,
    Shell::Fish,
    Shell::PowerShell,
    Shell::Elvish,
];

fn main() -> anyhow::Result<()> {
    run(&std::env::args().skip(1).collect::<Vec<_>>())
}

fn run(args: &[String]) -> anyhow::Result<()> {
    match args {
        [task, dir] if task == "assets" => assets(Path::new(dir)),
        _ => bail!("usage: cargo run -p wyrmyon-xtask -- assets <dir>"),
    }
}

fn assets(dir: &Path) -> anyhow::Result<()> {
    let man = dir.join("man");
    let completions = dir.join("completions");
    for sub in [&man, &completions] {
        std::fs::create_dir_all(sub).with_context(|| format!("creating {}", sub.display()))?;
    }
    for bin in BINARIES {
        let mut command = wyrmyon::command().name(bin).bin_name(bin);
        clap_mangen::generate_to(command.clone(), &man)?;
        for shell in SHELLS {
            clap_complete::generate_to(shell, &mut command, bin, &completions)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_binary_gets_man_pages_and_completions() {
        let dir = tempfile::tempdir().unwrap();
        assets(dir.path()).unwrap();
        for bin in BINARIES {
            let page = std::fs::read_to_string(dir.path().join(format!("man/{bin}.1"))).unwrap();
            assert!(page.contains(".TH ") && page.contains("receive"), "{page}");
            assert!(dir.path().join(format!("man/{bin}-send.1")).exists());
            for file in [
                format!("{bin}.bash"),
                format!("_{bin}"),
                format!("{bin}.fish"),
                format!("_{bin}.ps1"),
                format!("{bin}.elv"),
            ] {
                let script =
                    std::fs::read_to_string(dir.path().join("completions").join(&file)).unwrap();
                assert!(script.contains("accept-file"), "{file}");
            }
        }
    }

    #[test]
    fn the_command_line_names_the_task_and_its_directory() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out").to_string_lossy().into_owned();
        run(&["assets".to_owned(), out.clone()]).unwrap();
        assert!(Path::new(&out).join("man/wyrm.1").exists());
        for args in [
            vec![],
            vec!["assets".to_owned()],
            vec!["other".to_owned(), out],
        ] {
            assert!(run(&args).is_err());
        }
    }

    #[test]
    fn an_unwritable_directory_is_an_error() {
        let file = tempfile::NamedTempFile::new().unwrap();
        assert!(assets(file.path()).is_err());
    }
}
