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

pub fn home() -> Option<PathBuf> {
    crate::absolute(crate::cache_env().1)
}

pub fn run(args: ShimArgs) -> anyhow::Result<()> {
    let us = Us::current()?;
    let home = home();
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

pub fn hard_links(us: &Us, home: Option<&Path>) -> Vec<PathBuf> {
    us.shim_dirs(home)
        .into_iter()
        .map(|dir| dir.join(FILE))
        .filter(|link| {
            link.symlink_metadata()
                .is_ok_and(|m| !m.file_type().is_symlink())
                && us.owns(link)
        })
        .collect()
}

pub fn relink(us: &Us, links: &[PathBuf]) -> Vec<String> {
    links
        .iter()
        .filter_map(|link| relink_one(&us.target, link).err())
        .collect()
}

fn relink_one(target: &Path, link: &Path) -> Result<(), String> {
    let dir = link.parent().unwrap_or_else(|| Path::new("."));
    let aside = tempfile::Builder::new()
        .prefix(".wormhole-")
        .tempfile_in(dir)
        .map(tempfile::NamedTempFile::into_temp_path)
        .map_err(|e| format!("{} still runs the old wyrmyon: {e}", link.display()))?;
    std::fs::rename(link, &aside)
        .map_err(|e| format!("{} still runs the old wyrmyon: {e}", link.display()))?;
    let Err(e) = std::fs::hard_link(target, link) else {
        return Ok(());
    };
    if std::fs::rename(&aside, link).is_ok() {
        return Err(format!(
            "{} still runs the old wyrmyon: {e}",
            link.display()
        ));
    }
    let kept = aside.keep().unwrap_or_default();
    Err(format!(
        "{} is missing after the update ({e}): the old one is at {}, and `wyrm shim` links a new one",
        link.display(),
        kept.display()
    ))
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

#[cfg(test)]
mod tests {
    use super::*;

    struct Scene {
        root: tempfile::TempDir,
        us: Us,
    }

    impl Scene {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let bin = root.path().join("bin");
            std::fs::create_dir(&bin).unwrap();
            let exe = bin.join(PRIMARY);
            std::fs::write(&exe, b"wyrmyon").unwrap();
            executable(&exe);
            Self {
                us: Us::at(bin.join(SECONDARY)),
                root,
            }
        }

        fn dir(&self, name: &str) -> PathBuf {
            let dir = self.root.path().join(name);
            std::fs::create_dir_all(&dir).unwrap();
            dir
        }

        fn foreign(&self, dir: &str) -> PathBuf {
            let path = self.dir(dir).join(FILE);
            std::fs::write(&path, b"python").unwrap();
            executable(&path);
            path
        }
    }

    #[cfg(unix)]
    fn executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(not(unix))]
    fn executable(_: &Path) {}

    fn path_of(dirs: &[&Path]) -> OsString {
        std::env::join_paths(dirs).unwrap()
    }

    #[test]
    fn the_link_beside_the_binary_runs_it() {
        let scene = Scene::new();
        let bin = scene.us.dir();
        let path = path_of(&[&bin]);
        let said = create(&scene.us, &bin, &path).unwrap();
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(said[0].starts_with("Linked "), "{said:?}");
        let link = bin.join(FILE);
        assert!(scene.us.owns(&link));
        assert_eq!(std::fs::read(&link).unwrap(), b"wyrmyon");
        #[cfg(unix)]
        assert_eq!(std::fs::read_link(&link).unwrap(), Path::new(PRIMARY));

        let again = create(&scene.us, &bin, &path).unwrap();
        assert_eq!(again, [format!("{} already runs wyrmyon", link.display())]);
        let unlisted = create(&scene.us, &bin, &OsString::new()).unwrap();
        assert_eq!(
            unlisted[0],
            format!("{} already runs wyrmyon", link.display())
        );
        assert!(unlisted[1].starts_with("Add "), "{unlisted:?}");
    }

    #[test]
    fn a_link_elsewhere_says_when_its_directory_is_not_on_path() {
        let scene = Scene::new();
        let elsewhere = scene.root.path().join("home").join(".local").join("bin");
        let said = create(&scene.us, &elsewhere, &OsString::new()).unwrap();
        assert_eq!(
            said[1],
            format!("Add {} to your PATH to run wormhole.", elsewhere.display())
        );
        assert!(scene.us.owns(&elsewhere.join(FILE)));
        #[cfg(unix)]
        assert_eq!(
            std::fs::read_link(elsewhere.join(FILE)).unwrap(),
            scene.us.target
        );
    }

    #[test]
    fn another_wormhole_is_never_replaced() {
        let scene = Scene::new();
        let python = scene.foreign("python");
        let bin = scene.us.dir();
        let err = create(&scene.us, &bin, &path_of(&[&python.with_file_name("")]))
            .unwrap_err()
            .to_string();
        assert!(err.contains(&python.display().to_string()), "{err}");
        assert!(!bin.join(FILE).exists());

        let here = scene.foreign("here");
        let err = create(&scene.us, here.parent().unwrap(), &OsString::new())
            .unwrap_err()
            .to_string();
        assert_eq!(
            err,
            format!("{} is already there and is not wyrmyon", here.display())
        );
        assert_eq!(std::fs::read(&here).unwrap(), b"python");
    }

    #[cfg(unix)]
    #[test]
    fn only_a_runnable_wormhole_on_path_counts() {
        use std::os::unix::fs::PermissionsExt;
        let scene = Scene::new();
        let stray = scene.foreign("stray");
        std::fs::set_permissions(&stray, std::fs::Permissions::from_mode(0o644)).unwrap();
        let dangling = scene.dir("dangling").join(FILE);
        std::os::unix::fs::symlink("nowhere", &dangling).unwrap();
        let path = path_of(&[
            Path::new(""),
            stray.parent().unwrap(),
            dangling.parent().unwrap(),
        ]);
        assert_eq!(find(&path), None);
    }

    #[test]
    fn a_wormhole_on_path_that_runs_us_is_enough() {
        let scene = Scene::new();
        let bin = scene.us.dir();
        create(&scene.us, &bin, &OsString::new()).unwrap();
        let other = scene.dir("other");
        let said = create(&scene.us, &other, &path_of(&[&bin])).unwrap();
        assert_eq!(
            said,
            [format!("{} already runs wyrmyon", bin.join(FILE).display())]
        );
        assert!(!other.join(FILE).exists());
    }

    #[test]
    fn remove_takes_back_only_what_runs_us() {
        let scene = Scene::new();
        let bin = scene.us.dir();
        let home = scene.dir("home");
        let dirs = scene.us.shim_dirs(Some(&home));
        assert_eq!(dirs, [bin.clone(), home.join(".local").join("bin")]);
        assert_eq!(
            remove(&scene.us, &dirs).unwrap(),
            ["No wormhole link to wyrmyon to remove"]
        );
        create(&scene.us, &dirs[1], &OsString::new()).unwrap();
        let foreign = scene.foreign("python");
        let all = [foreign.parent().unwrap().to_path_buf(), dirs[1].clone()];
        assert_eq!(
            remove(&scene.us, &all).unwrap(),
            [format!("Removed {}", dirs[1].join(FILE).display())]
        );
        assert!(dirs[1].join(FILE).symlink_metadata().is_err());
        let err = remove(&scene.us, &all).unwrap_err().to_string();
        assert_eq!(
            err,
            format!("{} is not wyrmyon: left in place", foreign.display())
        );
        assert!(foreign.exists());
        assert_eq!(scene.us.shim_dirs(None), [bin]);
    }

    #[test]
    fn the_link_goes_beside_the_binary_when_it_can() {
        let scene = Scene::new();
        let home = scene.dir("home");
        assert_eq!(default_dir(&scene.us, Some(&home)).unwrap(), scene.us.dir());
        let gone = Us::at(scene.root.path().join("gone").join(PRIMARY));
        assert_eq!(
            default_dir(&gone, Some(&home)).unwrap(),
            home.join(".local").join("bin")
        );
        assert!(default_dir(&gone, None).is_err());
        assert_eq!(gone.target, scene.root.path().join("gone").join(PRIMARY));
    }

    #[test]
    fn an_update_relinks_a_hard_linked_shim() {
        let scene = Scene::new();
        let home = scene.dir("home");
        let local = scene.dir("home/.local/bin");
        let link = local.join(FILE);
        std::fs::hard_link(&scene.us.target, &link).unwrap();
        let elsewhere = scene.root.path().join("elsewhere");
        create(&scene.us, &elsewhere, &OsString::new()).unwrap();
        let links = hard_links(&scene.us, Some(&home));
        assert_eq!(links, std::slice::from_ref(&link));

        let staged = scene.us.dir().join("staged");
        std::fs::write(&staged, b"wyrmyon, updated").unwrap();
        std::fs::rename(&staged, &scene.us.target).unwrap();
        assert!(!scene.us.owns(&link));
        assert_eq!(relink(&scene.us, &links), Vec::<String>::new());
        assert!(scene.us.owns(&link));
        assert_eq!(std::fs::read(&link).unwrap(), b"wyrmyon, updated");
        assert_eq!(std::fs::read_dir(&local).unwrap().count(), 1);

        let missing = [scene.root.path().join("missing").join(FILE)];
        assert!(relink(&scene.us, &missing)[0].contains("still runs the old wyrmyon"));
    }

    #[test]
    fn this_binary_knows_itself() {
        let us = Us::current().unwrap();
        assert!(us.owns(&std::env::current_exe().unwrap()));
    }
}
