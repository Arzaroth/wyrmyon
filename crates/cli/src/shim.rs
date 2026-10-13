use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use clap::Args;

pub const FILE: &str = if cfg!(windows) {
    "wormhole.exe"
} else {
    "wormhole"
};
const PRIMARY: &str = if cfg!(windows) {
    "wyrmyon.exe"
} else {
    "wyrmyon"
};
const SECONDARY: &str = if cfg!(windows) { "wyrm.exe" } else { "wyrm" };

#[derive(Args)]
pub struct ShimArgs {
    /// Remove a wormhole link that runs wyrmyon, and nothing else
    #[arg(long)]
    remove: bool,
    /// Where the link goes: by default beside this binary when that is writable, else ~/.local/bin
    #[arg(long, value_name = "DIR")]
    dir: Option<PathBuf>,
}

pub struct Us {
    target: PathBuf,
    binaries: Vec<PathBuf>,
}

impl Us {
    pub fn current() -> anyhow::Result<Self> {
        let exe = std::env::current_exe().context("finding this binary")?;
        Ok(Self::at(exe))
    }

    fn at(exe: PathBuf) -> Self {
        let dir = exe.parent().map(Path::to_path_buf).unwrap_or_default();
        let primary = dir.join(PRIMARY);
        let target = if primary.is_file() {
            primary.clone()
        } else {
            exe.clone()
        };
        Self {
            target,
            binaries: vec![exe, primary, dir.join(SECONDARY)],
        }
    }

    pub fn owns(&self, path: &Path) -> bool {
        self.binaries
            .iter()
            .any(|ours| same_file::is_same_file(path, ours).unwrap_or(false))
    }

    fn dir(&self) -> PathBuf {
        self.target
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default()
    }

    pub fn shim_dirs(&self, home: Option<&Path>) -> Vec<PathBuf> {
        let mut dirs = vec![self.dir()];
        if let Some(home) = home {
            let local = home.join(".local").join("bin");
            if !dirs.contains(&local) {
                dirs.push(local);
            }
        }
        dirs
    }
}

pub fn run(args: ShimArgs) -> anyhow::Result<()> {
    let us = Us::current()?;
    let home = crate::absolute(crate::cache_env().1);
    let said = if args.remove {
        let dirs = args
            .dir
            .map_or_else(|| us.shim_dirs(home.as_deref()), |d| vec![d]);
        remove(&us, &dirs)?
    } else {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let dir = match args.dir {
            Some(dir) => dir,
            None => default_dir(&us, home.as_deref())?,
        };
        create(&us, &dir, &path)?
    };
    for line in said {
        println!("{line}");
    }
    Ok(())
}

fn default_dir(us: &Us, home: Option<&Path>) -> anyhow::Result<PathBuf> {
    let beside = us.dir();
    if writable(&beside) {
        return Ok(beside);
    }
    let home = home.context("this binary's directory is not writable, and there is no HOME")?;
    Ok(home.join(".local").join("bin"))
}

fn writable(dir: &Path) -> bool {
    tempfile::Builder::new()
        .prefix(".wyrmyon-shim")
        .tempfile_in(dir)
        .is_ok()
}

fn create(us: &Us, dir: &Path, path: &OsStr) -> anyhow::Result<Vec<String>> {
    if let Some(found) = find(path) {
        if us.owns(&found) {
            return Ok(vec![format!("{} already runs wyrmyon", found.display())]);
        }
        bail!(
            "{} is already on PATH and is not wyrmyon: remove it first",
            found.display()
        );
    }
    let link = dir.join(FILE);
    let mut said = Vec::new();
    if link.symlink_metadata().is_ok() {
        if !us.owns(&link) {
            bail!("{} is already there and is not wyrmyon", link.display());
        }
        said.push(format!("{} already runs wyrmyon", link.display()));
    } else {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        make_link(&us.target, &link)
            .with_context(|| format!("linking {} to wyrmyon", link.display()))?;
        said.push(format!(
            "Linked {} to {}: wormhole now runs wyrmyon",
            link.display(),
            us.target.display()
        ));
    }
    if !on_path(dir, path) {
        said.push(format!(
            "Add {} to your PATH to run wormhole.",
            dir.display()
        ));
    }
    Ok(said)
}

fn remove(us: &Us, dirs: &[PathBuf]) -> anyhow::Result<Vec<String>> {
    let mut said = Vec::new();
    let mut foreign = None;
    for dir in dirs {
        let link = dir.join(FILE);
        if link.symlink_metadata().is_err() {
            continue;
        }
        if us.owns(&link) {
            std::fs::remove_file(&link).with_context(|| format!("removing {}", link.display()))?;
            said.push(format!("Removed {}", link.display()));
        } else {
            foreign = Some(link);
        }
    }
    if said.is_empty() {
        if let Some(foreign) = foreign {
            bail!("{} is not wyrmyon: left in place", foreign.display());
        }
        said.push("No wormhole link to wyrmyon to remove".into());
    }
    Ok(said)
}

fn find(path: &OsStr) -> Option<PathBuf> {
    let names = names();
    std::env::split_paths(path)
        .filter(|dir| !dir.as_os_str().is_empty())
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|candidate| runnable(candidate))
}

#[cfg(windows)]
fn names() -> Vec<OsString> {
    let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    exts.split(';')
        .filter(|e| !e.is_empty())
        .map(|e| OsString::from(format!("wormhole{}", e.to_ascii_lowercase())))
        .collect()
}

#[cfg(not(windows))]
fn names() -> Vec<OsString> {
    vec![OsString::from("wormhole")]
}

#[cfg(unix)]
fn runnable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn runnable(path: &Path) -> bool {
    path.is_file()
}

fn on_path(dir: &Path, path: &OsStr) -> bool {
    let wanted = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    std::env::split_paths(path).any(|p| std::fs::canonicalize(&p).unwrap_or(p) == wanted)
}

#[cfg(unix)]
fn make_link(target: &Path, link: &Path) -> std::io::Result<()> {
    let real = |p: Option<&Path>| p.and_then(|p| std::fs::canonicalize(p).ok());
    let beside = real(target.parent()).is_some() && real(target.parent()) == real(link.parent());
    let to = match target.file_name() {
        Some(name) if beside => Path::new(name),
        _ => target,
    };
    std::os::unix::fs::symlink(to, link)
}

#[cfg(not(unix))]
fn make_link(target: &Path, link: &Path) -> std::io::Result<()> {
    std::fs::hard_link(target, link)
}
