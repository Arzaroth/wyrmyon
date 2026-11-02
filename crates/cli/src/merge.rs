use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};

use wyrmyon_transport_iroh::{Collection, Manifest, Placement};

use crate::printable;

pub fn clash(into: &Path, path: &Path, dir: bool, overwrite: bool) -> Option<String> {
    let in_the_way = |at: &Path| format!("{} is in the way", printable(&at.display().to_string()));
    for ancestor in path.ancestors().skip(1) {
        let at = into.join(ancestor);
        if !ancestor.as_os_str().is_empty() && at.symlink_metadata().is_ok() && !at.is_dir() {
            return Some(in_the_way(&at));
        }
    }
    let at = into.join(path);
    at.symlink_metadata().ok()?;
    match (dir, at.is_dir()) {
        (true, true) => None,
        (true, false) => Some(in_the_way(&at)),
        (false, false) if overwrite => None,
        (false, _) => Some(format!(
            "refusing to overwrite {}",
            printable(&at.display().to_string())
        )),
    }
}

const LISTED: usize = 20;

pub fn named(names: &[String]) -> Option<BTreeSet<String>> {
    let set: BTreeSet<String> = names
        .iter()
        .map(|name| name.strip_suffix('/').unwrap_or(name).to_owned())
        .collect();
    let usable = set
        .iter()
        .all(|name| crate::usable_name(name, cfg!(windows)) && !name.contains(['/', '\\']));
    (usable && !set.is_empty()).then_some(set)
}

pub fn listing(names: &[String]) -> String {
    use std::fmt::Write;
    let mut listed = String::new();
    for name in names.iter().take(LISTED) {
        let _ = writeln!(listed, "  {}", printable(name));
    }
    if names.len() > LISTED {
        let _ = writeln!(listed, "  and {} more", names.len() - LISTED);
    }
    listed
}

pub fn outside(names: &BTreeSet<String>, path: &Path) -> Option<String> {
    let first = path.components().next()?;
    let first = first.as_os_str().to_str()?;
    (!names.contains(first)).then(|| {
        format!(
            "{} is not under a name the sender offered",
            printable(&path.display().to_string())
        )
    })
}

pub fn clashing(
    into: &Path,
    manifest: &Manifest,
    overwrite: bool,
    names: &BTreeSet<String>,
) -> Option<String> {
    let files = manifest.files.iter().map(|f| (&f.path, false));
    files
        .chain(manifest.dirs.iter().map(|d| (d, true)))
        .find_map(|(path, dir)| {
            outside(names, Path::new(path)).or_else(|| clash(into, Path::new(path), dir, overwrite))
        })
}

pub async fn export(
    collection: Collection,
    into: &Path,
    overwrite: bool,
    names: BTreeSet<String>,
) -> anyhow::Result<()> {
    let mut merge = Merge::new(into, overwrite, names);
    collection.export_with(&mut merge).await?;
    merge.keep();
    Ok(())
}

pub struct Merge {
    into: PathBuf,
    overwrite: bool,
    names: BTreeSet<String>,
    dirs: Vec<PathBuf>,
    files: Vec<PathBuf>,
    partial: Option<PathBuf>,
    keep: bool,
}

impl Merge {
    pub fn new(into: &Path, overwrite: bool, names: BTreeSet<String>) -> Self {
        Self {
            into: into.to_owned(),
            overwrite,
            names,
            dirs: Vec::new(),
            files: Vec::new(),
            partial: None,
            keep: false,
        }
    }

    pub fn keep(mut self) {
        self.keep = true;
    }
}

impl Placement for Merge {
    fn dir(&mut self, path: &Path) -> io::Result<()> {
        if let Some(why) = outside(&self.names, path) {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, why));
        }
        let mut at = self.into.clone();
        for part in path.components() {
            at.push(part);
            if at.is_dir() {
                continue;
            }
            std::fs::create_dir(&at).map_err(|e| match e.kind() {
                io::ErrorKind::AlreadyExists => io::Error::new(
                    e.kind(),
                    format!("{} is in the way", printable(&at.display().to_string())),
                ),
                _ => e,
            })?;
            self.dirs.push(at.clone());
        }
        Ok(())
    }

    fn file(&mut self, path: &Path) -> io::Result<PathBuf> {
        if let Some(why) = outside(&self.names, path) {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, why));
        }
        if let Some(parent) = path.parent() {
            self.dir(parent)?;
        }
        if let Some(why) = clash(&self.into, path, false, self.overwrite) {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, why));
        }
        let target = self.into.join(path);
        let name = target.file_name().unwrap_or_default().to_string_lossy();
        let partial = target.with_file_name(format!(
            ".{name}.{}.wyrm-part",
            crate::receive::unique_suffix()
        ));
        self.partial = Some(partial.clone());
        Ok(partial)
    }

    fn placed(&mut self, path: &Path, written: &Path) -> io::Result<()> {
        let target = self.into.join(path);
        if self.overwrite && target.symlink_metadata().is_ok() {
            std::fs::rename(written, &target)?;
        } else {
            let appeared = || {
                io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!(
                        "{} appeared during the transfer",
                        printable(&target.display().to_string())
                    ),
                )
            };
            match std::fs::hard_link(written, &target) {
                Ok(()) => {
                    self.files.push(target);
                    std::fs::remove_file(written)?;
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Err(appeared()),
                Err(_) if target.symlink_metadata().is_ok() => return Err(appeared()),
                Err(_) => {
                    std::fs::rename(written, &target)?;
                    self.files.push(target);
                }
            }
        }
        self.partial = None;
        Ok(())
    }
}

impl Drop for Merge {
    fn drop(&mut self) {
        if let Some(partial) = &self.partial {
            let _ = std::fs::remove_file(partial);
        }
        if self.keep {
            return;
        }
        for file in self.files.iter().rev() {
            let _ = std::fs::remove_file(file);
        }
        for dir in self.dirs.iter().rev() {
            let _ = std::fs::remove_dir(dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn top() -> BTreeSet<String> {
        ["etc", "void", "a", "sub", "moved", "other"]
            .map(String::from)
            .into()
    }

    #[test]
    fn a_path_outside_the_offered_names_is_refused() {
        let into = tempfile::tempdir().unwrap();
        let mut merge = Merge::new(into.path(), false, top());
        let refused = merge.file(Path::new(".ssh/authorized_keys")).unwrap_err();
        assert!(
            refused.to_string().contains("not under a name"),
            "{refused}"
        );
        assert!(merge.dir(Path::new(".git/hooks")).is_err());
        assert_eq!(named(&["a.txt".into(), "photos/".into()]).unwrap().len(), 2);
        assert_eq!(named(&[]), None);
        assert_eq!(named(&["a/b".into()]), None);
        assert_eq!(named(&["..".into()]), None);
        let many: Vec<String> = (0..25).map(|n| format!("f{n}")).collect();
        let listed = listing(&many);
        assert!(
            listed.starts_with("  f0\n") && listed.ends_with("  and 5 more\n"),
            "{listed}"
        );
    }

    fn written(merge: &mut Merge, path: &str, contents: &[u8]) -> io::Result<()> {
        let partial = merge.file(Path::new(path))?;
        std::fs::write(&partial, contents)?;
        merge.placed(Path::new(path), &partial)
    }

    #[test]
    fn what_is_already_there_clashes_unless_it_is_a_directory_or_overwritten() {
        let into = tempfile::tempdir().unwrap();
        let into = into.path();
        std::fs::create_dir_all(into.join("etc/x")).unwrap();
        std::fs::write(into.join("etc/x/y.conf"), b"old").unwrap();
        std::fs::write(into.join("file"), b"old").unwrap();
        let clash = |path: &str, dir, overwrite| clash(into, Path::new(path), dir, overwrite);
        assert_eq!(clash("etc/x/new.conf", false, false), None);
        assert_eq!(clash("etc/x", true, false), None);
        assert_eq!(clash("etc/x/z/deeper", false, false), None);
        assert!(
            clash("etc/x/y.conf", false, false)
                .unwrap()
                .starts_with("refusing to overwrite")
        );
        assert_eq!(clash("etc/x/y.conf", false, true), None);
        assert!(
            clash("etc/x", false, true)
                .unwrap()
                .starts_with("refusing to overwrite")
        );
        assert!(
            clash("file", true, true)
                .unwrap()
                .ends_with("is in the way")
        );
        assert!(
            clash("file/below", false, true)
                .unwrap()
                .ends_with("is in the way")
        );
    }

    #[test]
    fn a_merge_fills_existing_directories_and_takes_back_only_what_it_added() {
        let into = tempfile::tempdir().unwrap();
        let into = into.path();
        std::fs::create_dir_all(into.join("etc/x")).unwrap();
        std::fs::write(into.join("etc/x/keep"), b"mine").unwrap();
        let mut merge = Merge::new(into, false, top());
        written(&mut merge, "etc/x/y.conf", b"new").unwrap();
        written(&mut merge, "etc/z/w.conf", b"new").unwrap();
        merge.dir(Path::new("void/inner")).unwrap();
        assert_eq!(std::fs::read(into.join("etc/z/w.conf")).unwrap(), b"new");
        let refused = written(&mut merge, "etc/x/keep", b"theirs").unwrap_err();
        assert!(refused.to_string().starts_with("refusing to overwrite"));
        drop(merge);
        let mut left: Vec<_> = walkdir(into);
        left.sort();
        assert_eq!(left, ["etc", "etc/x", "etc/x/keep"]);
        assert_eq!(std::fs::read(into.join("etc/x/keep")).unwrap(), b"mine");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_to_a_directory_counts_as_one_even_under_overwrite() {
        let into = tempfile::tempdir().unwrap();
        let into = into.path();
        std::fs::create_dir(into.join("real")).unwrap();
        std::os::unix::fs::symlink(into.join("real"), into.join("link")).unwrap();
        let refused = clash(into, Path::new("link"), false, true).unwrap();
        assert!(refused.starts_with("refusing to overwrite"), "{refused}");
        assert_eq!(clash(into, Path::new("link"), true, false), None);
    }

    #[test]
    fn a_kept_merge_stays_and_overwrites_in_place() {
        let into = tempfile::tempdir().unwrap();
        let into = into.path();
        std::fs::write(into.join("a"), b"old").unwrap();
        let mut merge = Merge::new(into, true, top());
        written(&mut merge, "a", b"new").unwrap();
        written(&mut merge, "sub/b", b"new").unwrap();
        merge.keep();
        assert_eq!(std::fs::read(into.join("a")).unwrap(), b"new");
        assert_eq!(std::fs::read(into.join("sub/b")).unwrap(), b"new");
        let mut left = walkdir(into);
        left.sort();
        assert_eq!(left, ["a", "sub", "sub/b"]);
    }

    #[test]
    fn a_file_in_the_way_of_a_directory_stops_the_merge() {
        let into = tempfile::tempdir().unwrap();
        let into = into.path();
        std::fs::write(into.join("etc"), b"a file").unwrap();
        let mut merge = Merge::new(into, true, top());
        let refused = merge.file(Path::new("etc/x")).unwrap_err();
        assert!(refused.to_string().ends_with("is in the way"), "{refused}");
        let partial = merge.file(Path::new("other")).unwrap();
        std::fs::write(&partial, b"half").unwrap();
        drop(merge);
        assert!(!partial.exists());
    }

    #[test]
    fn a_file_that_appears_meanwhile_is_kept_and_the_partial_dropped() {
        let into = tempfile::tempdir().unwrap();
        let into = into.path();
        let mut merge = Merge::new(into, false, top());
        let partial = merge.file(Path::new("a")).unwrap();
        std::fs::write(&partial, b"theirs").unwrap();
        std::fs::write(into.join("a"), b"mine").unwrap();
        let refused = merge.placed(Path::new("a"), &partial).unwrap_err();
        assert!(
            refused
                .to_string()
                .ends_with("appeared during the transfer")
        );
        drop(merge);
        assert_eq!(std::fs::read(into.join("a")).unwrap(), b"mine");
        assert!(!partial.exists());
    }

    #[test]
    fn what_cannot_be_hard_linked_is_moved_instead() {
        let into = tempfile::tempdir().unwrap();
        let into = into.path();
        let mut merge = Merge::new(into, false, top());
        let partial = merge.file(Path::new("moved")).unwrap();
        std::fs::create_dir(&partial).unwrap();
        merge.placed(Path::new("moved"), &partial).unwrap();
        merge.keep();
        assert!(into.join("moved").is_dir());
        assert!(!partial.exists());
    }

    #[test]
    fn a_merge_into_nothing_fails_without_leaving_anything() {
        let into = tempfile::tempdir().unwrap();
        let missing = into.path().join("missing");
        let mut merge = Merge::new(&missing, false, top());
        let failed = merge.file(Path::new("a/b")).unwrap_err();
        assert_eq!(failed.kind(), io::ErrorKind::NotFound);
        drop(merge);
        assert!(!missing.exists());
    }

    fn walkdir(root: &Path) -> Vec<String> {
        let mut found = Vec::new();
        let mut todo = vec![root.to_owned()];
        while let Some(dir) = todo.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                let relative = path.strip_prefix(root).unwrap();
                found.push(relative.to_string_lossy().replace('\\', "/"));
                if path.is_dir() {
                    todo.push(path);
                }
            }
        }
        found
    }
}
