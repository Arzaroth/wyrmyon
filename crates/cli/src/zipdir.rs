use std::fs::File;
use std::io::{self, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

pub struct Built {
    pub file: tempfile::NamedTempFile,
    pub zipsize: u64,
    pub numbytes: u64,
    pub numfiles: u64,
}

pub fn build(dir: &Path) -> anyhow::Result<Built> {
    let file = tempfile::NamedTempFile::new().context("creating a temporary zip file")?;
    let mut zip = ZipWriter::new(file.reopen().context("opening the temporary zip file")?);
    let mut totals = (0u64, 0u64);
    add_dir(&mut zip, dir, Path::new(""), &mut totals)?;
    zip.finish().context("writing the zip file")?;
    let zipsize = file.as_file().metadata()?.len();
    Ok(Built {
        file,
        zipsize,
        numbytes: totals.0,
        numfiles: totals.1,
    })
}

fn add_dir(
    zip: &mut ZipWriter<File>,
    dir: &Path,
    prefix: &Path,
    totals: &mut (u64, u64),
) -> anyhow::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .collect::<Result<_, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    if entries.is_empty() && !prefix.as_os_str().is_empty() {
        zip.add_directory(arcname(prefix)?, SimpleFileOptions::default())?;
    }
    for entry in entries {
        let path = entry.path();
        let name = prefix.join(entry.file_name());
        let link = entry.file_type()?.is_symlink();
        let meta =
            std::fs::metadata(&path).with_context(|| format!("reading {}", path.display()))?;
        if meta.is_dir() {
            if link {
                eprintln!("skipping {}: a symlink to a directory", path.display());
                continue;
            }
            add_dir(zip, &path, &name, totals)?;
        } else if meta.is_file() {
            let options = SimpleFileOptions::default()
                .compression_method(CompressionMethod::Deflated)
                .unix_permissions(meta.permissions().mode() & 0o777)
                .large_file(meta.len() >= u64::from(u32::MAX));
            zip.start_file(arcname(&name)?, options)?;
            let copied = io::copy(
                &mut File::open(&path).with_context(|| format!("opening {}", path.display()))?,
                zip,
            )?;
            totals.0 += copied;
            totals.1 += 1;
        } else {
            eprintln!("skipping {}: not a regular file", path.display());
        }
    }
    Ok(())
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

pub fn extract(zip_path: &Path, dest: &Path, limits: &Limits) -> anyhow::Result<()> {
    let mut archive =
        ZipArchive::new(File::open(zip_path).context("opening the received zip file")?)
            .context("the received data is not a zip file")?;
    let (mut bytes, mut files) = (0u64, 0u64);
    for index in 0..archive.len() {
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
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mode = entry.unix_mode().map_or(0o644, |m| m & 0o777);
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&target)
            .with_context(|| format!("creating {}", target.display()))?;
        let budget = limits.numbytes - bytes;
        let copied = io::copy(&mut (&mut entry).take(budget + 1), &mut out)?;
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

        let built = build(src.path()).unwrap();
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
        let built = build(src.path()).unwrap();
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
            assert!(extract(built.file.path(), dest.path(), &limits).is_err());
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
        assert!(extract(file.path(), &dest.path().join("inner"), &limits).is_err());
        assert!(!dest.path().join("escape").exists());
    }
}
