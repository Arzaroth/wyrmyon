use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, anyhow, bail};
use clap::Args;
use tokio::io::AsyncReadExt;
pub(crate) use wyrmyon_transfer::offer_text;
use wyrmyon_transfer::{
    Events, FileSource, Hashing, Source, Staged, require_iroh, send_data, send_over,
};
use wyrmyon_transport_iroh::{Manifest, Offered, Source as Member, Traffic};
use wyrmyon_wormhole::{Code, Wormhole};

use crate::discard::Sources;
use crate::protocol::{self, CollectionOffer, DirectoryOffer, FileOffer, Offer};
use crate::{CODE_LENGTH, Global, mood_for, printable, show_code, show_welcome, transfer, zipdir};

#[derive(Args)]
#[command(group = clap::ArgGroup::new("what").required(true).args(["text", "paths"]))]
pub struct SendArgs {
    /// Text to send; `-` reads it from stdin
    #[arg(long)]
    pub(crate) text: Option<String>,
    /// Files or directories to send; several travel as one bundle, and land side by side
    pub(crate) paths: Vec<PathBuf>,
    /// Keep each path as typed, not only its last name, like `rsync -R`
    #[arg(
        short = 'R',
        long,
        long_help = "Keep each path as typed, not only its last name, like `rsync -R` or `tar`. \
The leading `/` is dropped, a `/./` in the path marks where the kept part starts, \
`..` cannot be kept, and a trailing slash changes nothing.

  wyrm send a/b.txt              b.txt
  wyrm send -R a/b.txt           a/b.txt
  wyrm send -R /etc/x/y.conf     etc/x/y.conf
  wyrm send -R /etc/./x/y.conf   x/y.conf
  wyrm send -R photos/           photos/... (the same as photos)
  wyrm send -R photos/./         the files inside photos, without photos/
  wyrm send -R ../a.txt          refused: use /./ to say what to keep

Another wyrm shows the top names before accepting, then puts the kept paths \
under its current directory, or under the directory `-o` names: \
`wyrm receive -o / CODE` puts etc/x/y.conf back at /etc/x/y.conf. It creates \
the directories that are missing, uses the ones already there, and refuses a \
file that is already there unless it was given --overwrite. Any other \
receiver, the Python client included, gets them inside a directory named files."
    )]
    pub(crate) relative: bool,
    /// Number of words in the generated code
    #[arg(long, default_value_t = CODE_LENGTH, value_parser = clap::value_parser!(u8).range(1..=8))]
    pub(crate) code_length: u8,
    /// Use this code instead of generating one, e.g. the one a receiver chose
    #[arg(long, conflicts_with = "code_length")]
    pub(crate) code: Option<String>,
    /// Also show the code as a QR code, for a phone to scan
    #[arg(long)]
    pub(crate) qr: bool,
    /// Serve this many receivers with the same code, one after another
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..))]
    pub(crate) times: u32,
    /// Delete the paths once every receiver has confirmed it got them
    #[arg(
        long,
        conflicts_with = "text",
        long_help = "Delete the paths once every receiver has confirmed it got them. \
A refused, failed or interrupted transfer deletes nothing, and with --times \
one failed receiver keeps everything. A path that changed during the transfer \
is kept. A link goes, not what it points at. The current directory, the home \
directory and their parents are refused."
    )]
    pub(crate) delete_source: bool,
    #[arg(skip)]
    pub(crate) literal_text: bool,
    #[arg(skip)]
    pub(crate) as_wormhole: bool,
}

enum Payload {
    Text(String),
    File(Arc<dyn Source>, FileOffer),
    Tree(String, Arc<Vec<zipdir::Entry>>, bool),
}

fn staged_for(payload: &Payload, times: u32, force_classic: bool) -> Staged {
    let mut staged = Staged {
        several: times > 1,
        ..Staged::default()
    };
    if !force_classic {
        hash_ahead(&mut staged, payload);
    }
    staged
}

fn hash_ahead(staged: &mut Staged, payload: &Payload) {
    match payload {
        Payload::Text(_) => {}
        Payload::File(source, _) => {
            staged.blob_ahead = Some(Hashing::blob(Arc::clone(source)));
        }
        Payload::Tree(_, entries, _) => {
            staged.collection_ahead = Some(collection_hashing(entries));
        }
    }
}

fn collection_hashing(entries: &[zipdir::Entry]) -> Hashing<(Offered, Manifest)> {
    let (mut members, mut dirs, mut total) = (Vec::new(), Vec::new(), 0);
    for entry in entries {
        match entry {
            zipdir::Entry::File {
                source,
                name,
                mode,
                size,
            } => {
                total += size;
                members.push(Member {
                    path: source.clone(),
                    name: name.clone(),
                    mode: *mode,
                });
            }
            zipdir::Entry::EmptyDir(dir) if dir.is_empty() => {}
            zipdir::Entry::EmptyDir(dir) => dirs.push(dir.clone()),
        }
    }
    Hashing::collection(members, dirs, total)
}

impl SendArgs {
    pub fn of_paths(paths: Vec<PathBuf>) -> Self {
        Self {
            text: None,
            paths,
            relative: false,
            code_length: CODE_LENGTH,
            code: None,
            qr: false,
            times: 1,
            delete_source: false,
            literal_text: false,
            as_wormhole: false,
        }
    }

    pub fn of_stdin() -> Self {
        Self {
            text: Some("-".into()),
            paths: Vec::new(),
            relative: false,
            code_length: CODE_LENGTH,
            code: None,
            qr: false,
            times: 1,
            delete_source: false,
            literal_text: false,
            as_wormhole: false,
        }
    }
}

pub async fn run(global: &Global, args: SendArgs) -> anyhow::Result<()> {
    let code: Option<Code> = args.code.as_deref().map(str::parse).transpose()?;
    let sources = if args.delete_source {
        Some(Sources::take(&args.paths)?)
    } else {
        None
    };
    let payload = match args.text {
        Some(text) if text == "-" && !args.literal_text => Payload::Text(stdin_text().await?),
        Some(text) => Payload::Text(text),
        None => paths_payload(args.paths, args.relative).await?,
    };

    let qr = args.qr.then_some((global.ascii, crate::qr::Role::Follower));
    let times = args.times;
    let config = global.config();
    if times > 1 && code.as_ref().is_some_and(|code| !code.is_reusable()) {
        bail!(
            "--times needs a code on a nameplate of 7 digits or more, which the server never hands out; leave out --code to get one"
        );
    }
    let given = code.or_else(|| (times > 1).then(|| Code::reusable(usize::from(args.code_length))));
    let mut pending = if let Some(code) = given {
        show_code(&code, qr, &other_side(&code, args.as_wormhole));
        wyrmyon_wormhole::join(&config, code).await?
    } else {
        let pending = wyrmyon_wormhole::create(&config, usize::from(args.code_length)).await?;
        show_code(
            pending.code(),
            qr,
            &other_side(pending.code(), args.as_wormhole),
        );
        pending
    };
    show_welcome(pending.welcome());
    if times > 1 {
        eprintln!("This code serves {times} receivers, one after another. A wrong code ends it.");
    }
    if let Some(sources) = &sources {
        sources.announce(times);
    }

    let code = pending.code().clone();
    let mut staged = staged_for(&payload, times, global.force_classic);
    let mut zip = None;
    let (mut round, mut failed) = (1, 0);
    let outcome = loop {
        let mut wormhole = match pending.pair().await {
            Ok(wormhole) => wormhole,
            Err(e) if times > 1 => {
                break Err(anyhow!(e).context(format!("receiver {round} of {times}")));
            }
            Err(e) => break Err(e.into()),
        };
        let next = if round < times {
            Some(wyrmyon_wormhole::join(&config, code.clone()).await)
        } else {
            None
        };
        let result = deliver(&mut wormhole, global, &payload, &mut staged, &mut zip).await;
        wormhole.close(mood_for(&result)).await;
        if times == 1 {
            break result;
        }
        match result {
            Ok(()) => eprintln!("Receiver {round} of {times}: done"),
            Err(e) => {
                failed += 1;
                eprintln!(
                    "Receiver {round} of {times}: {}",
                    printable(&format!("{e:#}"))
                );
            }
        }
        round += 1;
        pending = match next {
            None if failed == 0 => break Ok(()),
            None => break Err(anyhow!("{failed} of {times} transfers failed")),
            Some(Ok(pending)) => pending,
            Some(Err(e)) => break Err(anyhow!(e).context(format!("receiver {round} of {times}"))),
        };
        eprintln!("Waiting for receiver {round} of {times}..");
    };
    staged.close().await;
    match (outcome, sources) {
        (Ok(()), Some(sources)) => sources.discard().await,
        (Err(e), Some(_)) => {
            if times > 1 {
                eprintln!("Nothing deleted: not every receiver confirmed");
            } else {
                eprintln!("Nothing deleted: the transfer did not go through");
            }
            Err(e)
        }
        (outcome, None) => outcome,
    }
}

async fn deliver(
    wormhole: &mut Wormhole,
    global: &Global,
    payload: &Payload,
    staged: &mut Staged,
    zip: &mut Option<zipdir::Built>,
) -> anyhow::Result<()> {
    let events = transfer::terminal(global, "Sending");
    let traffic = match payload {
        Payload::Text(text) => return send_text(wormhole, text.clone()).await,
        Payload::File(source, offer) => {
            send_data(
                wormhole,
                &global.options(),
                staged,
                Offer::File(offer.clone()),
                source,
                &events,
            )
            .await?
        }
        Payload::Tree(name, entries, bundle) => {
            send_tree(
                wormhole, global, staged, zip, name, entries, *bundle, &events,
            )
            .await?
        }
    };
    eprintln!("Confirmation received. Transfer complete.");
    if let Some(went) = traffic.and_then(transfer::went) {
        eprintln!("{went}");
    }
    Ok(())
}

fn other_side(code: &Code, as_wormhole: bool) -> String {
    if as_wormhole {
        format!("  wormhole receive {code}\n\n(or: wyrm {code})")
    } else {
        format!("  wyrm {code}\n\n(or: wormhole receive {code})")
    }
}

async fn stdin_text() -> anyhow::Result<String> {
    let mut text = String::new();
    tokio::io::stdin()
        .read_to_string(&mut text)
        .await
        .context("reading stdin")?;
    Ok(text)
}

async fn paths_payload(mut paths: Vec<PathBuf>, relative: bool) -> anyhow::Result<Payload> {
    if paths.len() == 1 && !relative {
        return path_payload(paths.remove(0)).await;
    }
    let entries = listed(move |cancel| zipdir::list_bundle(&paths, relative, cancel)).await?;
    Ok(Payload::Tree("files".into(), Arc::new(entries), true))
}

async fn listed(
    list: impl FnOnce(&zipdir::Cancel) -> anyhow::Result<Vec<zipdir::Entry>> + Send + 'static,
) -> anyhow::Result<Vec<zipdir::Entry>> {
    let cancel = zipdir::Cancel::default();
    let _cancel_on_drop = cancel.on_drop();
    tokio::task::spawn_blocking(move || list(&cancel))
        .await
        .context("reading the files to send")?
}

async fn path_payload(path: PathBuf) -> anyhow::Result<Payload> {
    let meta = tokio::fs::metadata(&path)
        .await
        .with_context(|| format!("cannot send {}", path.display()))?;
    let name = display_name(&path)?;
    if meta.is_dir() {
        let entries = listed(move |cancel| zipdir::list(&path, cancel)).await?;
        return Ok(Payload::Tree(name, Arc::new(entries), false));
    }
    if !meta.is_file() {
        bail!("{} is not a regular file", path.display());
    }
    eprintln!("Sending {} bytes file named '{name}'", meta.len());
    let offer = FileOffer {
        filename: name,
        filesize: meta.len(),
    };
    let size = offer.filesize;
    Ok(Payload::File(Arc::new(FileSource::new(path, size)), offer))
}

fn top_names(entries: &[zipdir::Entry]) -> Vec<String> {
    let named = entries.iter().filter_map(|entry| match entry {
        zipdir::Entry::File { name, .. } | zipdir::Entry::EmptyDir(name) => {
            (!name.is_empty()).then_some(name)
        }
    });
    let top: std::collections::BTreeSet<String> = named
        .map(|name| match name.split_once('/') {
            Some((first, _)) => format!("{first}/"),
            None => name.clone(),
        })
        .collect();
    top.into_iter().collect()
}

pub fn kept(path: &Path) -> anyhow::Result<PathBuf> {
    let text = path
        .to_str()
        .with_context(|| format!("{} is not valid UTF-8", path.display()))?;
    let markers: &[&str] = if cfg!(windows) {
        &["/./", "\\.\\", "/.\\", "\\./"]
    } else {
        &["/./"]
    };
    let tail = markers
        .iter()
        .filter_map(|m| text.find(m).map(|at| at + m.len()))
        .min()
        .map_or(text, |from| &text[from..]);
    let mut kept = PathBuf::new();
    for part in Path::new(tail).components() {
        match part {
            Component::Normal(name) => kept.push(name),
            Component::ParentDir => bail!(
                "-R cannot keep `..` in {}: mark where the kept path starts with /./",
                path.display()
            ),
            Component::Prefix(_) | Component::RootDir | Component::CurDir => {}
        }
    }
    Ok(kept)
}

pub fn display_name(path: &Path) -> anyhow::Result<String> {
    let resolved =
        std::fs::canonicalize(path).with_context(|| format!("cannot send {}", path.display()))?;
    Ok(resolved
        .file_name()
        .with_context(|| format!("{} has no name", path.display()))?
        .to_string_lossy()
        .into_owned())
}

async fn send_text(wormhole: &mut Wormhole, text: String) -> anyhow::Result<()> {
    offer_text(wormhole, text).await?;
    eprintln!("text message sent");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn send_tree(
    wormhole: &mut Wormhole,
    global: &Global,
    staged: &mut Staged,
    zip: &mut Option<zipdir::Built>,
    name: &str,
    entries: &Arc<Vec<zipdir::Entry>>,
    bundle: bool,
    events: &Arc<dyn Events>,
) -> anyhow::Result<Option<Traffic>> {
    if global.use_collection(wormhole) {
        return send_collection(wormhole, global, staged, name, entries, bundle, events).await;
    }
    staged.drop_collection_ahead().await;
    require_iroh(wormhole, &global.options()).await?;
    if zip.is_none() {
        eprintln!("Building zipfile..");
        let cancel = zipdir::Cancel::default();
        let _cancel_on_drop = cancel.on_drop();
        let entries = Arc::clone(entries);
        let zipped = tokio::task::spawn_blocking(move || zipdir::zip(&entries, &cancel))
            .await
            .context("building the zip file")
            .and_then(|built| built);
        match zipped {
            Ok(built) => *zip = Some(built),
            Err(e) => {
                protocol::send_error(wormhole, "the sender could not read its data").await?;
                return Err(e);
            }
        }
    }
    let built = zip.as_ref().expect("the zip file was just built");
    let bundle = bundle && crate::speaks_iroh(wormhole);
    if bundle {
        eprintln!(
            "Sending {} files ({} bytes compressed)",
            built.numfiles, built.zipsize
        );
    } else {
        eprintln!(
            "Sending directory ({} bytes compressed) named '{name}'",
            built.zipsize
        );
    }
    let offer = DirectoryOffer {
        mode: "zipfile/deflated".into(),
        dirname: name.to_owned(),
        zipsize: built.zipsize,
        numbytes: built.numbytes,
        numfiles: built.numfiles,
        bundle,
        names: if bundle {
            top_names(entries)
        } else {
            Vec::new()
        },
    };
    let source: Arc<dyn Source> =
        Arc::new(FileSource::new(built.file.path().to_owned(), offer.zipsize));
    send_data(
        wormhole,
        &global.options(),
        staged,
        Offer::Directory(offer),
        &source,
        events,
    )
    .await
}

async fn send_collection(
    wormhole: &mut Wormhole,
    global: &Global,
    staged: &mut Staged,
    name: &str,
    entries: &[zipdir::Entry],
    bundle: bool,
    events: &Arc<dyn Events>,
) -> anyhow::Result<Option<Traffic>> {
    if staged.collection.is_none() {
        let hashing = staged
            .collection_ahead
            .take()
            .unwrap_or_else(|| collection_hashing(entries));
        let (offered, manifest) = match hashing.finish(events.as_ref()).await {
            Ok(imported) => imported,
            Err(e) => {
                protocol::send_error(wormhole, "the sender could not read its data").await?;
                return Err(e);
            }
        };
        let offer = CollectionOffer {
            name: name.to_owned(),
            numbytes: manifest.numbytes(),
            numfiles: manifest.files.len() as u64,
            bundle,
            names: if bundle {
                top_names(entries)
            } else {
                Vec::new()
            },
        };
        staged.collection = Some((offered, offer));
    }
    let (offered, offer) = staged
        .collection
        .as_ref()
        .expect("the collection was just imported");
    if offer.bundle {
        eprintln!(
            "Sending {} files ({} bytes)",
            offer.numfiles, offer.numbytes
        );
    } else {
        eprintln!(
            "Sending directory ({} files, {} bytes) named '{}'",
            offer.numfiles, offer.numbytes, offer.name
        );
    }
    send_over(
        wormhole,
        &global.options(),
        Offer::Collection(offer.clone()),
        None,
        offer.numbytes,
        Some(offered),
        events,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_keeps_the_path_as_typed_from_the_marker_on() {
        let kept =
            |path: &str| kept(Path::new(path)).map(|p| p.to_string_lossy().replace('\\', "/"));
        for (typed, landing) in [
            ("a/b.txt", "a/b.txt"),
            ("/etc/x/y.conf", "etc/x/y.conf"),
            ("/etc/./x/y.conf", "x/y.conf"),
            ("./a/./b/c", "b/c"),
            ("photos/", "photos"),
            ("photos/./", ""),
            (".././a.txt", "a.txt"),
        ] {
            assert_eq!(kept(typed).unwrap(), landing, "{typed}");
        }
        let refused = kept("../a.txt").unwrap_err().to_string();
        assert!(refused.contains("cannot keep `..`"), "{refused}");
        assert!(kept("a/./../b").is_err());
    }
}
