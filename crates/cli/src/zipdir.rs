use std::fs::File;
use std::io::{self, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, bail};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

pub struct Built {
    pub file: tempfile::NamedTempFile,
    pub zipsize: u64,
    pub numbytes: u64,
    pub numfiles: u64,
}

const ZIP64_FROM: u64 = 0xF000_0000;

#[derive(Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn check(&self) -> anyhow::Result<()> {
        if self.0.load(Ordering::Relaxed) {
            bail!("interrupted");
        }
        Ok(())
    }

    pub fn on_drop(&self) -> CancelOnDrop {
        CancelOnDrop(self.clone())
    }
}

pub struct CancelOnDrop(Cancel);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.0.store(true, Ordering::Relaxed);
    }
}

struct Checked<'a> {
    inner: &'a mut dyn Read,
    cancel: &'a Cancel,
}

impl Read for Checked<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.cancel.0.load(Ordering::Relaxed) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "interrupted"));
        }
        self.inner.read(buf)
    }
}

pub fn build(dir: &Path, cancel: &Cancel) -> anyhow::Result<Built> {
    zip_with(cancel, |walk, zip| walk.add_dir(zip, dir, Path::new("")))
}

pub fn build_bundle(paths: &[PathBuf], cancel: &Cancel) -> anyhow::Result<Built> {
    let mut named = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for path in paths {
        let name = std::fs::canonicalize(path)
            .with_context(|| format!("cannot send {}", path.display()))?
            .file_name()
            .with_context(|| format!("{} has no name", path.display()))?
            .to_owned();
        if !seen.insert(name.clone()) {
            bail!("two of the paths are named {}", name.to_string_lossy());
        }
        named.push((path, PathBuf::from(name)));
    }
    zip_with(cancel, |walk, zip| {
        named
            .iter()
            .try_for_each(|(path, name)| walk.add_entry(zip, path, name))
    })
}

fn zip_with(
    cancel: &Cancel,
    add: impl FnOnce(&mut Walk<'_>, &mut ZipWriter<File>) -> anyhow::Result<()>,
) -> anyhow::Result<Built> {
    let file = tempfile::NamedTempFile::new().context("creating a temporary zip file")?;
    let mut zip = ZipWriter::new(file.reopen().context("opening the temporary zip file")?);
    let mut walk = Walk {
        totals: (0, 0),
        ancestors: Vec::new(),
        cancel,
    };
    add(&mut walk, &mut zip)?;
    let totals = walk.totals;
    zip.finish().context("writing the zip file")?;
    let zipsize = file.as_file().metadata()?.len();
    Ok(Built {
        file,
        zipsize,
        numbytes: totals.0,
        numfiles: totals.1,
    })
}

struct Walk<'a> {
    totals: (u64, u64),
    ancestors: Vec<PathBuf>,
    cancel: &'a Cancel,
}

impl Walk<'_> {
    fn add_dir(
        &mut self,
        zip: &mut ZipWriter<File>,
        dir: &Path,
        prefix: &Path,
    ) -> anyhow::Result<()> {
        let real =
            std::fs::canonicalize(dir).with_context(|| format!("reading {}", dir.display()))?;
        if self.ancestors.contains(&real) {
            eprintln!("skipping {}: a symlink loop", dir.display());
            return Ok(());
        }
        self.ancestors.push(real);
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .with_context(|| format!("reading {}", dir.display()))?
            .collect::<Result<_, _>>()?;
        entries.sort_by_key(std::fs::DirEntry::file_name);
        if entries.is_empty() {
            let name = if prefix.as_os_str().is_empty() {
                "./".to_owned()
            } else {
                arcname(prefix)?
            };
            zip.add_directory(name, SimpleFileOptions::default())?;
        }
        for entry in entries {
            self.add_entry(zip, &entry.path(), &prefix.join(entry.file_name()))?;
        }
        self.ancestors.pop();
        Ok(())
    }

    fn add_entry(
        &mut self,
        zip: &mut ZipWriter<File>,
        path: &Path,
        name: &Path,
    ) -> anyhow::Result<()> {
        self.cancel.check()?;
        let meta =
            std::fs::metadata(path).with_context(|| format!("reading {}", path.display()))?;
        if meta.is_dir() {
            self.add_dir(zip, path, name)?;
        } else if meta.is_file() {
            let options = SimpleFileOptions::default()
                .compression_method(CompressionMethod::Deflated)
                .unix_permissions(meta.permissions().mode() & 0o777)
                .large_file(meta.len() >= ZIP64_FROM);
            zip.start_file(arcname(name)?, options)?;
            let mut source =
                File::open(path).with_context(|| format!("opening {}", path.display()))?;
            let mut reader = Checked {
                inner: &mut source,
                cancel: self.cancel,
            };
            let copied = io::copy(&mut reader, zip)?;
            self.totals.0 += copied;
            self.totals.1 += 1;
        } else {
            eprintln!("skipping {}: not a regular file", path.display());
        }
        Ok(())
    }
}

fn arcname(path: &Path) -> anyhow::Result<String> {
    let parts: Option<Vec<&str>> = path.iter().map(|p| p.to_str()).collect();
    parts
        .map(|p| p.join("/"))
        .with_context(|| format!("{} is not valid UTF-8", path.display()))
}

pub struct Limits {
    pub numbytes: u64,
    pub numfiles: u64,
}

pub fn extract(
    zip_path: &Path,
    dest: &Path,
    limits: &Limits,
    cancel: &Cancel,
) -> anyhow::Result<()> {
    let mut archive =
        ZipArchive::new(File::open(zip_path).context("opening the received zip file")?)
            .context("the received data is not a zip file")?;
    let (mut bytes, mut files) = (0u64, 0u64);
    for index in 0..archive.len() {
        cancel.check()?;
        let mut entry = archive.by_index(index)?;
        let Some(relative) = entry.enclosed_name() else {
            bail!(
                "the zip file has an entry outside its directory: {}",
                crate::printable(entry.name())
            );
        };
        let target = dest.join(&relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&target)?;
            continue;
        }
        if entry.is_symlink() {
            eprintln!("skipping {}: a symlink", relative.display());
            continue;
        }
        files += 1;
        if files > limits.numfiles {
            bail!(
                "the directory has more files than the {} offered",
                limits.numfiles
            );
        }
        std::fs::create_dir_all(target.parent().unwrap_or(dest))?;
        let mode = entry.unix_mode().map_or(0o644, |m| m & 0o777);
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&target)
            .with_context(|| format!("creating {}", target.display()))?;
        let budget = limits.numbytes - bytes;
        let mut entry = (&mut entry).take(budget.saturating_add(1));
        let mut limited = Checked {
            inner: &mut entry,
            cancel,
        };
        let copied = io::copy(&mut limited, &mut out)?;
        if copied > budget {
            bail!(
                "the directory holds more than the {} bytes offered",
                limits.numbytes
            );
        }
        bytes += copied;
        out.flush()?;
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(mode))?;
    }
    Ok(())
}

pub struct NewDir {
    path: PathBuf,
    keep: bool,
}

impl NewDir {
    pub fn create(path: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir(path).with_context(|| format!("creating {}", path.display()))?;
        Ok(Self {
            path: path.to_owned(),
            keep: false,
        })
    }

    pub fn keep(mut self) {
        self.keep = true;
    }
}

impl Drop for NewDir {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_directory_round_trips_through_zip() {
        let src = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(src.path().join("sub/empty")).unwrap();
        std::fs::write(src.path().join("a.txt"), b"alpha").unwrap();
        std::fs::write(src.path().join("sub/b.bin"), vec![7u8; 100_000]).unwrap();
        std::fs::set_permissions(
            src.path().join("a.txt"),
            std::fs::Permissions::from_mode(0o750),
        )
        .unwrap();

        let built = build(src.path(), &Cancel::default()).unwrap();
        assert_eq!((built.numbytes, built.numfiles), (100_005, 2));
        assert!(built.zipsize < 100_005);

        let out = tempfile::tempdir().unwrap();
        let dest = out.path().join("copy");
        std::fs::create_dir(&dest).unwrap();
        extract(
            built.file.path(),
            &dest,
            &Limits {
                numbytes: 100_005,
                numfiles: 2,
            },
            &Cancel::default(),
        )
        .unwrap();
        assert_eq!(std::fs::read(dest.join("a.txt")).unwrap(), b"alpha");
        assert_eq!(
            std::fs::read(dest.join("sub/b.bin")).unwrap(),
            vec![7u8; 100_000]
        );
        assert!(dest.join("sub/empty").is_dir());
        let mode = std::fs::metadata(dest.join("a.txt"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o750);
    }

    #[test]
    fn extraction_stops_at_the_offered_size_and_count() {
        let src = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("a"), vec![1u8; 1000]).unwrap();
        std::fs::write(src.path().join("b"), vec![1u8; 1000]).unwrap();
        let built = build(src.path(), &Cancel::default()).unwrap();
        for limits in [
            Limits {
                numbytes: 1500,
                numfiles: 2,
            },
            Limits {
                numbytes: 2000,
                numfiles: 1,
            },
        ] {
            let dest = tempfile::tempdir().unwrap();
            assert!(extract(built.file.path(), dest.path(), &limits, &Cancel::default()).is_err());
        }
    }

    #[test]
    fn entries_outside_the_directory_are_refused() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let mut zip = ZipWriter::new(file.reopen().unwrap());
        zip.start_file("../escape", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"x").unwrap();
        zip.finish().unwrap();
        let dest = tempfile::tempdir().unwrap();
        let limits = Limits {
            numbytes: 10,
            numfiles: 10,
        };
        assert!(
            extract(
                file.path(),
                &dest.path().join("inner"),
                &limits,
                &Cancel::default()
            )
            .is_err()
        );
        assert!(!dest.path().join("escape").exists());
    }

    fn limits() -> Limits {
        Limits {
            numbytes: u64::MAX,
            numfiles: 100,
        }
    }

    #[test]
    fn directory_symlinks_are_followed_but_loops_are_not() {
        let src = tempfile::tempdir().unwrap();
        std::fs::create_dir(src.path().join("real")).unwrap();
        std::fs::write(src.path().join("real/x"), b"x").unwrap();
        std::os::unix::fs::symlink("real", src.path().join("link")).unwrap();
        std::os::unix::fs::symlink("..", src.path().join("real/up")).unwrap();
        let built = build(src.path(), &Cancel::default()).unwrap();
        assert_eq!(built.numfiles, 2);
        let dest = tempfile::tempdir().unwrap();
        extract(
            built.file.path(),
            dest.path(),
            &limits(),
            &Cancel::default(),
        )
        .unwrap();
        assert_eq!(std::fs::read(dest.path().join("link/x")).unwrap(), b"x");
        assert!(!dest.path().join("real/up").exists());
    }

    #[test]
    fn an_empty_directory_still_has_an_entry() {
        let src = tempfile::tempdir().unwrap();
        let built = build(src.path(), &Cancel::default()).unwrap();
        let archive = ZipArchive::new(built.file.reopen().unwrap()).unwrap();
        assert_eq!(archive.len(), 1);
        let dest = tempfile::tempdir().unwrap();
        let inner = dest.path().join("inner");
        std::fs::create_dir(&inner).unwrap();
        extract(built.file.path(), &inner, &limits(), &Cancel::default()).unwrap();
        assert!(inner.is_dir());
    }

    #[test]
    fn symlink_entries_are_skipped() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let mut zip = ZipWriter::new(file.reopen().unwrap());
        zip.add_symlink("evil", "/etc/passwd", SimpleFileOptions::default())
            .unwrap();
        zip.finish().unwrap();
        let dest = tempfile::tempdir().unwrap();
        extract(file.path(), dest.path(), &limits(), &Cancel::default()).unwrap();
        assert!(dest.path().join("evil").symlink_metadata().is_err());
    }

    #[test]
    fn cancelling_stops_the_work_and_drops_the_new_directory() {
        let src = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("a"), b"a").unwrap();
        let cancel = Cancel::default();
        drop(cancel.on_drop());
        assert!(build(src.path(), &cancel).is_err());

        let built = build(src.path(), &Cancel::default()).unwrap();
        let dest = tempfile::tempdir().unwrap();
        let target = dest.path().join("t");
        let made = NewDir::create(&target).unwrap();
        assert!(extract(built.file.path(), &target, &limits(), &cancel).is_err());
        drop(made);
        assert!(!target.exists());
    }

    #[test]
    fn a_cancelled_read_stops_mid_file() {
        let cancel = Cancel::default();
        let mut data = &b"data"[..];
        let mut reader = Checked {
            inner: &mut data,
            cancel: &cancel,
        };
        let mut buf = [0u8; 2];
        assert_eq!(reader.read(&mut buf).unwrap(), 2);
        drop(cancel.on_drop());
        assert!(reader.read(&mut buf).is_err());
    }

    #[test]
    fn sockets_and_other_special_files_are_left_out() {
        let src = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("kept"), b"k").unwrap();
        let _socket = std::os::unix::net::UnixListener::bind(src.path().join("sock")).unwrap();
        let built = build(src.path(), &Cancel::default()).unwrap();
        assert_eq!(built.numfiles, 1);
    }
}
