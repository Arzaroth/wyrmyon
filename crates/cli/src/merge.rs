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
    let meta = at.symlink_metadata().ok()?;
    match (dir, at.is_dir()) {
        (true, true) => None,
        (true, false) => Some(in_the_way(&at)),
        (false, _) if overwrite && !meta.is_dir() => None,
        (false, _) => Some(format!(
            "refusing to overwrite {}",
            printable(&at.display().to_string())
        )),
    }
}

pub fn clashing(into: &Path, manifest: &Manifest, overwrite: bool) -> Option<String> {
    let files = manifest.files.iter().map(|f| (&f.path, false));
    files
        .chain(manifest.dirs.iter().map(|d| (d, true)))
        .find_map(|(path, dir)| clash(into, Path::new(path), dir, overwrite))
}

pub async fn export(collection: Collection, into: &Path, overwrite: bool) -> anyhow::Result<()> {
    let mut merge = Merge::new(into, overwrite);
    collection.export_with(&mut merge).await?;
    merge.keep();
    Ok(())
}

pub struct Merge {
    into: PathBuf,
    overwrite: bool,
    dirs: Vec<PathBuf>,
    files: Vec<PathBuf>,
    partial: Option<PathBuf>,
    keep: bool,
}

impl Merge {
    pub fn new(into: &Path, overwrite: bool) -> Self {
        Self {
            into: into.to_owned(),
            overwrite,
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
