use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, bail};

use crate::printable;

#[derive(Debug, PartialEq)]
enum Seen {
    File(u64, Option<SystemTime>),
    Dir,
    Link(PathBuf),
    Other,
}

struct Source {
    path: PathBuf,
    seen: Vec<(PathBuf, Seen)>,
}

pub(crate) struct Sources(Vec<Source>);

impl Sources {
    pub(crate) fn take(paths: &[PathBuf]) -> anyhow::Result<Self> {
        let cwd = std::env::current_dir().context("finding the current directory")?;
        Self::take_from(paths, &cwd, std::env::home_dir().as_deref())
    }

    fn take_from(paths: &[PathBuf], cwd: &Path, home: Option<&Path>) -> anyhow::Result<Self> {
        let cwd = fs::canonicalize(cwd).context("finding the current directory")?;
        let home = home.and_then(|home| fs::canonicalize(home).ok());
        let mut sources = Vec::with_capacity(paths.len());
        for typed in paths {
            let path = target(typed);
            let real =
                located(&path).with_context(|| format!("cannot send {}", typed.display()))?;
            if cwd.starts_with(&real) {
                bail!(
                    "--delete-source refuses {}: it is or holds the current directory",
                    typed.display()
                );
            }
            if home.as_ref().is_some_and(|home| home.starts_with(&real)) {
                bail!(
                    "--delete-source refuses {}: it is or holds the home directory",
                    typed.display()
                );
            }
            let seen =
                snapshot(&path).with_context(|| format!("cannot send {}", typed.display()))?;
            sources.push(Source { path, seen });
        }
        Ok(Self(sources))
    }

    pub(crate) fn announce(&self, times: u32) {
        let names = self
            .0
            .iter()
            .map(|source| printable(&source.path.display().to_string()))
            .collect::<Vec<_>>()
            .join(", ");
        if times > 1 {
            eprintln!("Once all {times} receivers confirm, this deletes: {names}");
        } else {
            eprintln!("Once the receiver confirms, this deletes: {names}");
        }
    }

    pub(crate) async fn discard(self) -> anyhow::Result<()> {
        tokio::task::spawn_blocking(move || self.discard_now())
            .await
            .context("deleting the sources")?
    }

    fn discard_now(self) -> anyhow::Result<()> {
        let total = self.0.len();
        let mut kept = 0;
        for source in self.0 {
            let name = printable(&source.path.display().to_string());
            match snapshot(&source.path) {
                Ok(now) if now == source.seen => match remove(&source.path) {
                    Ok(()) => eprintln!("Deleted {name}"),
                    Err(e) => {
                        kept += 1;
                        eprintln!("Could not delete {name}: {e}");
                    }
                },
                Ok(_) => {
                    kept += 1;
                    eprintln!("Kept {name}: it changed during the transfer");
                }
                Err(e) => {
                    kept += 1;
                    eprintln!("Kept {name}: {e}");
                }
            }
        }
        if kept > 0 {
            bail!("the transfer went through, but {kept} of {total} paths were not deleted");
        }
        Ok(())
    }
}

fn target(typed: &Path) -> PathBuf {
    match (typed.parent(), typed.file_name()) {
        (Some(parent), Some(name)) => parent.join(name),
        _ => typed.to_owned(),
    }
}

fn located(path: &Path) -> io::Result<PathBuf> {
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) if fs::symlink_metadata(path)?.is_symlink() => {
            Ok(fs::canonicalize(Path::new(".").join(parent))?.join(name))
        }
        _ => fs::canonicalize(path),
    }
}

fn snapshot(path: &Path) -> io::Result<Vec<(PathBuf, Seen)>> {
    let mut seen = Vec::new();
    walk(path, Path::new(""), &mut seen)?;
    Ok(seen)
}

fn walk(path: &Path, rel: &Path, seen: &mut Vec<(PathBuf, Seen)>) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    let kind = if meta.is_symlink() {
        Seen::Link(fs::read_link(path)?)
    } else if meta.is_dir() {
        Seen::Dir
    } else if meta.is_file() {
        Seen::File(meta.len(), meta.modified().ok())
    } else {
        Seen::Other
    };
    let dir = kind == Seen::Dir;
    seen.push((rel.to_owned(), kind));
    if dir {
        let mut names = fs::read_dir(path)?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<io::Result<Vec<_>>>()?;
        names.sort();
        for name in names {
            walk(&path.join(&name), &rel.join(&name), seen)?;
        }
    }
    Ok(())
}

fn remove(path: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path).or_else(|e| fs::remove_dir(path).map_err(|_| e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn taken(dir: &Path, names: &[&str]) -> anyhow::Result<Sources> {
        let paths: Vec<PathBuf> = names.iter().map(|name| dir.join(name)).collect();
        let elsewhere = tempfile::tempdir().unwrap();
        Sources::take_from(&paths, elsewhere.path(), None)
    }

    #[test]
    fn an_unchanged_file_and_directory_are_deleted() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "a").unwrap();
        fs::create_dir_all(dir.path().join("d/e")).unwrap();
        fs::write(dir.path().join("d/e/f.txt"), "f").unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let sources = Sources::take_from(
            &[dir.path().join("a.txt"), dir.path().join("d/./")],
            elsewhere.path(),
            None,
        )
        .unwrap();
        sources.announce(1);
        sources.announce(2);
        sources.discard_now().unwrap();
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn a_path_that_changed_or_vanished_is_kept_and_reported() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["grew.txt", "gone.txt", "same.txt"] {
            fs::write(dir.path().join(name), "x").unwrap();
        }
        fs::create_dir(dir.path().join("d")).unwrap();
        let sources = taken(dir.path(), &["grew.txt", "gone.txt", "same.txt", "d"]).unwrap();
        fs::write(dir.path().join("grew.txt"), "xx").unwrap();
        fs::remove_file(dir.path().join("gone.txt")).unwrap();
        fs::write(dir.path().join("d/new.txt"), "n").unwrap();
        let refused = sources.discard_now().unwrap_err().to_string();
        assert!(refused.contains("3 of 4 paths"), "{refused}");
        assert!(dir.path().join("grew.txt").exists());
        assert!(dir.path().join("d/new.txt").exists());
        assert!(!dir.path().join("same.txt").exists());
    }

    #[test]
    fn the_current_and_home_directories_and_their_parents_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let inner = dir.path().join("inner");
        fs::create_dir(&inner).unwrap();
        for typed in [
            inner.clone(),
            dir.path().to_owned(),
            inner.join(".."),
            "/".into(),
        ] {
            let refused = Sources::take_from(std::slice::from_ref(&typed), &inner, None)
                .err()
                .unwrap()
                .to_string();
            assert!(
                refused.contains("current directory"),
                "{typed:?}: {refused}"
            );
        }
        let elsewhere = tempfile::tempdir().unwrap();
        let refused = Sources::take_from(&[dir.path().to_owned()], elsewhere.path(), Some(&inner))
            .err()
            .unwrap()
            .to_string();
        assert!(refused.contains("home directory"), "{refused}");
        assert!(Sources::take_from(&[dir.path().join("missing")], elsewhere.path(), None).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_link_goes_but_not_what_it_points_at() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("real")).unwrap();
        fs::write(dir.path().join("real/kept.txt"), "k").unwrap();
        std::os::unix::fs::symlink("real", dir.path().join("link")).unwrap();
        std::os::unix::fs::symlink("real/kept.txt", dir.path().join("file-link")).unwrap();
        let sources = taken(dir.path(), &["link/", "file-link"]).unwrap();
        sources.discard_now().unwrap();
        assert!(!dir.path().join("link").exists());
        assert!(fs::symlink_metadata(dir.path().join("file-link")).is_err());
        assert!(dir.path().join("real/kept.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_link_is_located_by_its_own_name() {
        let dir = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(dir.path(), dir.path().join("here")).unwrap();
        assert_eq!(
            located(&dir.path().join("here")).unwrap(),
            fs::canonicalize(dir.path()).unwrap().join("here")
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_path_that_cannot_be_deleted_is_reported() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let locked = dir.path().join("locked");
        fs::create_dir(&locked).unwrap();
        fs::write(locked.join("f.txt"), "f").unwrap();
        let sources = taken(&locked, &["f.txt"]).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();
        let root = fs::File::create(locked.join("probe")).is_ok();
        let refused = sources.discard_now();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        if root {
            return;
        }
        assert!(refused.unwrap_err().to_string().contains("1 of 1"));
        assert!(locked.join("f.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_device_is_seen_as_something_else() {
        assert_eq!(
            snapshot(Path::new("/dev/null")).unwrap(),
            [(PathBuf::new(), Seen::Other)]
        );
    }
}
