use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{BufRead, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use clap::{Args, Subcommand};
use sha2::{Digest, Sha256};
use wyrmyon_wormhole::{Code, Config, Wormhole};

use crate::protocol::{self, Answer, AppMessage, Offer};
use crate::receive::refuse;
use crate::{CODE_LENGTH, Global, home, mood_for, send, show_welcome};

pub const APPID: &str = "lothar.com/wormhole/ssh-add";

const MAX_LINE: usize = 16 * 1024;

const KINDS: [&str; 8] = [
    "ssh-ed25519",
    "ssh-rsa",
    "ssh-dss",
    "ecdsa-sha2-nistp256",
    "ecdsa-sha2-nistp384",
    "ecdsa-sha2-nistp521",
    "sk-ssh-ed25519@openssh.com",
    "sk-ecdsa-sha2-nistp256@openssh.com",
];

#[derive(Args)]
pub struct SshArgs {
    #[command(subcommand)]
    pub(crate) action: Action,
    #[arg(skip)]
    pub(crate) appid: Option<String>,
    #[arg(skip)]
    pub(crate) as_wormhole: bool,
}

#[derive(Subcommand)]
pub enum Action {
    /// Wait for an SSH public key and add it to an `authorized_keys` file
    Invite(InviteArgs),
    /// Send your SSH public key, in answer to `ssh invite`
    Accept(AcceptArgs),
}

#[derive(Args)]
pub struct InviteArgs {
    /// Number of words in the code
    #[arg(
        short,
        long,
        value_name = "NUMWORDS",
        default_value_t = CODE_LENGTH,
        value_parser = clap::value_parser!(u8).range(1..=8)
    )]
    pub(crate) code_length: u8,
    /// Add to the `authorized_keys` of USER (Unix only)
    #[arg(short, long, value_name = "USER")]
    pub(crate) user: Option<String>,
}

#[derive(Args)]
pub struct AcceptArgs {
    /// The code from `ssh invite`
    pub(crate) code: String,
    /// The public key file to send, or a directory to look for one in; by default `~/.ssh`
    #[arg(short = 'F', long, value_name = "PATH")]
    pub(crate) key_file: Option<PathBuf>,
    /// Send the key without asking
    #[arg(short, long)]
    pub(crate) yes: bool,
}

pub async fn run(global: &Global, args: SshArgs) -> anyhow::Result<()> {
    let appid = args.appid.unwrap_or_else(|| APPID.to_owned());
    match args.action {
        Action::Invite(invite_args) => {
            let target = Target::of(invite_args.user.as_deref(), home())?;
            invite(
                global,
                appid,
                invite_args.code_length,
                &target,
                args.as_wormhole,
            )
            .await
        }
        Action::Accept(accept_args) => {
            let code: Code = accept_args.code.parse()?;
            let key = asked(accept_args.key_file, accept_args.yes).await?;
            accept(global, appid, code, &key).await
        }
    }
}

fn hint(code: &Code, as_wormhole: bool) -> String {
    let (first, then) = if as_wormhole {
        ("wormhole", "wyrm")
    } else {
        ("wyrm", "wormhole")
    };
    format!(
        "Now tell the other user to run:\n\n{first} ssh accept {code}\n\n(or: {then} ssh accept {code})\n"
    )
}

async fn asked(key_file: Option<PathBuf>, yes: bool) -> anyhow::Result<PublicKey> {
    let (answered, answer) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        crate::receive::drop_typed_ahead();
        let _ = answered.send(chosen_key(
            key_file.as_deref(),
            yes,
            home(),
            &mut std::io::stdin().lock(),
            &mut std::io::stdout(),
        ));
    });
    answer.await.context("choosing the key")?
}

fn config(global: &Global, appid: String) -> Config {
    Config {
        relay_url: global.relay_url.clone(),
        appid,
        ..Config::default()
    }
}

async fn invite(
    global: &Global,
    appid: String,
    code_length: u8,
    target: &Target,
    as_wormhole: bool,
) -> anyhow::Result<()> {
    target.check(&mut std::io::stdout())?;
    let pending =
        wyrmyon_wormhole::create(&config(global, appid), usize::from(code_length)).await?;
    println!("{}", hint(pending.code(), as_wormhole));
    show_welcome(pending.welcome());
    let mut wormhole = pending.pair().await?;
    let result = take_key(&mut wormhole, target).await;
    wormhole.close(mood_for(&result)).await;
    let key = result?;
    println!(
        "Appended key type='{}' id='{}' to '{}'",
        key.kind,
        key.id(),
        target.keys().display()
    );
    println!("Fingerprint: {}", key.fingerprint());
    Ok(())
}

async fn take_key(wormhole: &mut Wormhole, target: &Target) -> anyhow::Result<PublicKey> {
    let text = loop {
        match protocol::next(wormhole, "sender").await? {
            AppMessage::Offer(Offer::Message(text)) => break text,
            AppMessage::Offer(_) => {
                return refuse(wormhole, "the invitation takes an SSH public key only").await;
            }
            _ => {}
        }
    };
    let Ok(key) = PublicKey::parse(&text) else {
        return refuse(wormhole, "that is not one SSH public key").await;
    };
    if let Err(e) = target.append(&key) {
        let _ = protocol::send_error(wormhole, "the receiver could not write the key").await;
        return Err(e);
    }
    protocol::send(
        wormhole,
        &AppMessage::Answer(Answer::MessageAck("ok".into())),
    )
    .await?;
    Ok(key)
}

async fn accept(global: &Global, appid: String, code: Code, key: &PublicKey) -> anyhow::Result<()> {
    let pending = wyrmyon_wormhole::join(&config(global, appid), code).await?;
    show_welcome(pending.welcome());
    let mut wormhole = pending.pair().await?;
    let result = send::offer_text(&mut wormhole, key.line()).await;
    wormhole.close(mood_for(&result)).await;
    result?;
    println!("Key sent.");
    Ok(())
}

fn chosen_key(
    key_file: Option<&Path>,
    yes: bool,
    home: Option<PathBuf>,
    input: &mut dyn BufRead,
    output: &mut dyn Write,
) -> anyhow::Result<PublicKey> {
    let path = find_key(key_file, home, input, output)?;
    let text =
        fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;
    let key = PublicKey::parse(&text).with_context(|| format!("cannot send {}", path.display()))?;
    writeln!(
        output,
        "Sending public key type='{}' keyid='{}'",
        key.kind,
        key.id()
    )?;
    if !yes {
        write!(output, "Really send public key '{}' ? [y/N]: ", key.id())?;
        output.flush()?;
        let mut answer = String::new();
        input.read_line(&mut answer).context("reading the answer")?;
        if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            bail!("the key was not sent");
        }
    }
    Ok(key)
}

fn find_key(
    key_file: Option<&Path>,
    home: Option<PathBuf>,
    input: &mut dyn BufRead,
    output: &mut dyn Write,
) -> anyhow::Result<PathBuf> {
    let dir = match key_file {
        Some(file) if !file.is_dir() => return Ok(file.to_owned()),
        Some(dir) => dir.to_owned(),
        None => home
            .context("no absolute HOME to find ~/.ssh in; use --key-file")?
            .join(".ssh"),
    };
    let mut keys: Vec<PathBuf> = fs::read_dir(&dir)
        .with_context(|| format!("cannot read {}", dir.display()))?
        .filter_map(|entry| Some(entry.ok()?.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "pub") && path.is_file())
        .collect();
    keys.sort();
    match keys.as_slice() {
        [] => bail!("no public keys in {}", dir.display()),
        [one] => Ok(one.clone()),
        several => Ok(several[pick(several, input, output)?].clone()),
    }
}

fn pick(
    keys: &[PathBuf],
    input: &mut dyn BufRead,
    output: &mut dyn Write,
) -> anyhow::Result<usize> {
    writeln!(output, "Multiple public-keys found:")?;
    for (i, key) in keys.iter().enumerate() {
        let name = key.file_name().unwrap_or_default().to_string_lossy();
        writeln!(output, "  {i}: {name}")?;
    }
    loop {
        write!(output, "Send which one?: ")?;
        output.flush()?;
        let mut answer = String::new();
        if input.read_line(&mut answer).context("reading the answer")? == 0 {
            bail!("no key was chosen");
        }
        if let Ok(i) = answer.trim().parse::<usize>()
            && i < keys.len()
        {
            return Ok(i);
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct PublicKey {
    kind: String,
    blob: String,
    comment: String,
    digest: [u8; 32],
}

impl PublicKey {
    fn parse(text: &str) -> anyhow::Result<Self> {
        let line = text.trim();
        if line.len() > MAX_LINE {
            bail!("too long for a public key");
        }
        if line
            .chars()
            .any(|c| c != '\t' && (c.is_control() || invisible(c)))
        {
            bail!("not one line of visible text");
        }
        let mut fields = line.splitn(2, char::is_whitespace);
        let kind = fields.next().unwrap_or_default();
        let rest = fields.next().unwrap_or_default().trim_start();
        let (blob, comment) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        if !KINDS.contains(&kind) {
            bail!("not a known SSH public key type");
        }
        let decoded = STANDARD.decode(blob).context("not base64")?;
        if named_kind(&decoded) != Some(kind.as_bytes()) {
            bail!("the key data is not of the type it says");
        }
        Ok(Self {
            kind: kind.to_owned(),
            blob: blob.to_owned(),
            comment: comment.trim().to_owned(),
            digest: Sha256::digest(&decoded).into(),
        })
    }

    fn id(&self) -> &str {
        if self.comment.is_empty() {
            "unknown"
        } else {
            &self.comment
        }
    }

    fn line(&self) -> String {
        format!("{} {} {}", self.kind, self.blob, self.comment)
            .trim_end()
            .to_owned()
    }

    fn fingerprint(&self) -> String {
        format!("SHA256:{}", STANDARD_NO_PAD.encode(self.digest))
    }
}

fn invisible(c: char) -> bool {
    matches!(
        c,
        '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}'
    )
}

fn named_kind(blob: &[u8]) -> Option<&[u8]> {
    let (len, rest) = blob.split_first_chunk::<4>()?;
    rest.get(..usize::try_from(u32::from_be_bytes(*len)).ok()?)
}

#[derive(Debug)]
struct Target {
    dir: PathBuf,
    owner: Option<(u32, u32)>,
}

impl Target {
    fn of(user: Option<&str>, home: Option<PathBuf>) -> anyhow::Result<Self> {
        match user {
            None => Ok(Self {
                dir: home
                    .context("no absolute HOME to find ~/.ssh in; use --user")?
                    .join(".ssh"),
                owner: None,
            }),
            Some(user) => {
                let (home, uid, gid) =
                    account(user)?.with_context(|| format!("no user named {user}"))?;
                Self::of_account(user, &home, uid, gid)
            }
        }
    }

    fn of_account(user: &str, home: &Path, uid: u32, gid: u32) -> anyhow::Result<Self> {
        if !home.is_absolute() {
            bail!("{user} has no absolute home");
        }
        Ok(Self {
            dir: home.join(".ssh"),
            owner: Some((uid, gid)),
        })
    }

    fn keys(&self) -> PathBuf {
        self.dir.join("authorized_keys")
    }

    fn check(&self, output: &mut dyn Write) -> anyhow::Result<()> {
        let keys = self.keys();
        if keys.exists() {
            let mut options = OpenOptions::new();
            options.append(true);
            if self.owner.is_some() {
                no_link_no_wait(&mut options);
            }
            options
                .open(&keys)
                .with_context(|| format!("no write permission on {}", keys.display()))?;
        } else {
            writeln!(
                output,
                "Note: '{}' not found; will be created",
                keys.display()
            )?;
            let mut within = self.dir.as_path();
            if !self.dir.exists() {
                writeln!(
                    output,
                    "      '{}' doesn't exist either",
                    self.dir.display()
                )?;
                within = self.dir.parent().unwrap_or(within);
            }
            if !writable(within) {
                bail!(
                    "cannot create {}: no write permission on {}",
                    keys.display(),
                    within.display()
                );
            }
        }
        if self.dir.exists() {
            fs::read_dir(&self.dir)
                .with_context(|| format!("cannot read {}", self.dir.display()))?;
        }
        Ok(())
    }

    fn append(&self, key: &PublicKey) -> anyhow::Result<()> {
        let path = self.keys();
        let mut file = match self.owner {
            Some(owner) => owned_keys(&self.dir, owner)?,
            None => self.own_keys()?,
        };
        let mut line = String::new();
        if !ends_a_line(&mut file).with_context(|| format!("cannot read {}", path.display()))? {
            line.push('\n');
        }
        line.push_str(&key.line());
        line.push('\n');
        file.write_all(line.as_bytes())
            .with_context(|| format!("cannot write {}", path.display()))?;
        Ok(())
    }

    fn own_keys(&self) -> anyhow::Result<fs::File> {
        if !self.dir.exists() {
            let mut dir = DirBuilder::new();
            private_dir(&mut dir);
            dir.create(&self.dir)
                .with_context(|| format!("cannot create {}", self.dir.display()))?;
        }
        let path = self.keys();
        let mut options = OpenOptions::new();
        options.read(true).append(true).create(true);
        private_file(&mut options);
        options
            .open(&path)
            .with_context(|| format!("cannot write {}", path.display()))
    }
}

#[cfg(unix)]
fn owned_keys(dir: &Path, (uid, gid): (u32, u32)) -> anyhow::Result<fs::File> {
    use rustix::fs::{CWD, Mode, OFlags, openat};
    use std::os::unix::fs::fchown;
    let mut builder = DirBuilder::new();
    private_dir(&mut builder);
    let made = match builder.create(dir) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => false,
        Err(e) => return Err(e).with_context(|| format!("cannot create {}", dir.display())),
    };
    let unlinked = |path: &Path| format!("cannot open {} without following a link", path.display());
    let nofollow = OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let ssh = openat(
        CWD,
        dir,
        OFlags::RDONLY | OFlags::DIRECTORY | nofollow,
        Mode::empty(),
    )
    .with_context(|| unlinked(dir))?;
    let ssh = fs::File::from(ssh);
    if made {
        fchown(&ssh, Some(uid), Some(gid))
            .with_context(|| format!("cannot hand over {}", dir.display()))?;
    }
    theirs(&ssh, uid, dir)?;
    let path = dir.join("authorized_keys");
    let write = OFlags::RDWR | OFlags::APPEND | nofollow;
    let new = OFlags::CREATE | OFlags::EXCL;
    let (keys, created) = match openat(&ssh, "authorized_keys", write | new, Mode::from(0o600)) {
        Ok(keys) => (keys, true),
        Err(rustix::io::Errno::EXIST) => (
            openat(&ssh, "authorized_keys", write, Mode::empty())
                .with_context(|| unlinked(&path))?,
            false,
        ),
        Err(e) => return Err(e).with_context(|| format!("cannot write {}", path.display())),
    };
    let keys = fs::File::from(keys);
    if created {
        fchown(&keys, Some(uid), Some(gid))
            .with_context(|| format!("cannot hand over {}", path.display()))?;
    }
    theirs(&keys, uid, &path)?;
    Ok(keys)
}

#[cfg(unix)]
fn theirs(file: &fs::File, uid: u32, path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let meta = file
        .metadata()
        .with_context(|| format!("cannot read {}", path.display()))?;
    let plain = meta.is_dir() || (meta.is_file() && meta.nlink() == 1);
    if meta.uid() != uid || !plain {
        bail!(
            "{} is not the user's own plain file, or has other names: not writing to it",
            path.display()
        );
    }
    Ok(())
}

#[cfg(not(unix))]
fn owned_keys(_: &Path, _: (u32, u32)) -> anyhow::Result<fs::File> {
    bail!("--user works on Unix only")
}

fn ends_a_line(file: &mut fs::File) -> std::io::Result<bool> {
    if file.metadata()?.len() == 0 {
        return Ok(true);
    }
    file.seek(SeekFrom::End(-1))?;
    let mut last = [0];
    file.read_exact(&mut last)?;
    Ok(last == *b"\n")
}

#[cfg(unix)]
fn account(user: &str) -> anyhow::Result<Option<(PathBuf, u32, u32)>> {
    let found =
        nix::unistd::User::from_name(user).with_context(|| format!("cannot look up {user}"))?;
    Ok(found.map(|user| (user.dir, user.uid.as_raw(), user.gid.as_raw())))
}

#[cfg(not(unix))]
fn account(_: &str) -> anyhow::Result<Option<(PathBuf, u32, u32)>> {
    bail!("--user works on Unix only")
}

#[cfg(unix)]
fn no_link_no_wait(options: &mut OpenOptions) {
    use rustix::fs::OFlags;
    use std::os::unix::fs::OpenOptionsExt;
    options.custom_flags((OFlags::NOFOLLOW | OFlags::NONBLOCK).bits().cast_signed());
}

#[cfg(not(unix))]
fn no_link_no_wait(_: &mut OpenOptions) {}

#[cfg(unix)]
fn writable(dir: &Path) -> bool {
    rustix::fs::access(dir, rustix::fs::Access::WRITE_OK).is_ok()
}

#[cfg(not(unix))]
fn writable(_: &Path) -> bool {
    true
}

#[cfg(unix)]
fn private_dir(dir: &mut DirBuilder) {
    use std::os::unix::fs::DirBuilderExt;
    dir.mode(0o700);
}

#[cfg(not(unix))]
fn private_dir(_: &mut DirBuilder) {}

#[cfg(unix)]
fn private_file(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600);
}

#[cfg(not(unix))]
fn private_file(_: &mut OpenOptions) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn blob(kind: &str) -> String {
        let mut raw = u32::try_from(kind.len()).unwrap().to_be_bytes().to_vec();
        raw.extend_from_slice(kind.as_bytes());
        raw.extend_from_slice(&[7; 32]);
        STANDARD.encode(raw)
    }

    fn key_text(comment: &str) -> String {
        format!("ssh-ed25519 {} {comment}", blob("ssh-ed25519"))
    }

    fn key(comment: &str) -> PublicKey {
        PublicKey::parse(&key_text(comment)).unwrap()
    }

    fn target(dir: &Path) -> Target {
        Target {
            dir: dir.join(".ssh"),
            owner: None,
        }
    }

    fn choose(
        home: &Path,
        key_file: Option<&Path>,
        yes: bool,
        typed: &str,
    ) -> (anyhow::Result<PublicKey>, String) {
        let mut said = Vec::new();
        let key = chosen_key(
            key_file,
            yes,
            Some(home.to_owned()),
            &mut typed.as_bytes(),
            &mut said,
        );
        (key, String::from_utf8(said).unwrap())
    }

    #[test]
    fn a_public_key_is_read_field_by_field() {
        let parsed = PublicKey::parse(&format!("  {}\n", key_text("me@host"))).unwrap();
        assert_eq!(
            (parsed.kind.as_str(), parsed.id()),
            ("ssh-ed25519", "me@host")
        );
        assert_eq!(parsed.line(), key_text("me@host"));
        assert!(parsed.fingerprint().starts_with("SHA256:"));
        assert!(!parsed.fingerprint().ends_with('='));
        let spaced = format!("ssh-ed25519\t{}   two words ", blob("ssh-ed25519"));
        assert_eq!(PublicKey::parse(&spaced).unwrap().id(), "two words");
        let bare = PublicKey::parse(&format!("ssh-rsa {}", blob("ssh-rsa"))).unwrap();
        assert_eq!(bare.id(), "unknown");
        assert_eq!(bare.line(), format!("ssh-rsa {}", blob("ssh-rsa")));
        for kind in KINDS {
            assert!(
                PublicKey::parse(&format!("{kind} {}", blob(kind))).is_ok(),
                "{kind}"
            );
        }
    }

    #[test]
    fn anything_but_one_public_key_is_not_a_key() {
        let good = key_text("a");
        for bad in [
            String::new(),
            "ssh-ed25519".to_owned(),
            "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----"
                .to_owned(),
            format!("{good}\n{good}"),
            format!("{good}\rsecond"),
            format!("command=\"sh\" {good}"),
            format!("no-pty {good}"),
            format!("ssh-ed25519 {} x", blob("ssh-rsa")),
            "ssh-ed25519 not*base64".to_owned(),
            format!("ssh-ed25519 {}", STANDARD.encode([0, 0])),
            format!("ssh-ed25519 {}", STANDARD.encode([0, 0, 0, 99, 1])),
            format!("{good}\u{1b}[2J"),
            format!("{good} admin\u{202e}tsoh@"),
            format!("{good} a\u{2028}b"),
            format!("{good} {}", "x".repeat(MAX_LINE)),
        ] {
            assert!(PublicKey::parse(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_missing_ssh_directory_is_made_private() {
        let home = tempfile::tempdir().unwrap();
        let target = target(home.path());
        let mut said = Vec::new();
        target.check(&mut said).unwrap();
        let said = String::from_utf8(said).unwrap();
        assert!(said.contains("not found; will be created"), "{said}");
        assert!(said.contains("doesn't exist either"), "{said}");
        target.append(&key("one")).unwrap();
        assert_eq!(
            fs::read_to_string(target.keys()).unwrap(),
            format!("{}\n", key_text("one"))
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&target.dir), 0o700);
            assert_eq!(mode(&target.keys()), 0o600);
        }
    }

    #[test]
    fn keys_are_appended_on_lines_of_their_own() {
        let home = tempfile::tempdir().unwrap();
        let target = target(home.path());
        fs::create_dir(&target.dir).unwrap();
        fs::write(target.keys(), "old key without a newline").unwrap();
        let mut said = Vec::new();
        target.check(&mut said).unwrap();
        assert_eq!(said, b"");
        target.append(&key("one")).unwrap();
        target.append(&key("two")).unwrap();
        assert_eq!(
            fs::read_to_string(target.keys()).unwrap(),
            format!(
                "old key without a newline\n{}\n{}\n",
                key_text("one"),
                key_text("two")
            )
        );
    }

    #[test]
    fn an_unusable_destination_is_found_before_the_network() {
        let home = tempfile::tempdir().unwrap();
        let target = target(home.path());
        fs::create_dir_all(target.keys()).unwrap();
        let err = target.check(&mut Vec::new()).unwrap_err();
        assert!(format!("{err}").contains("no write permission"), "{err}");
        assert!(target.append(&key("x")).is_err());

        let file = tempfile::tempdir().unwrap();
        fs::write(file.path().join(".ssh"), "").unwrap();
        let blocked = Target {
            dir: file.path().join(".ssh"),
            owner: None,
        };
        let err = blocked.check(&mut Vec::new()).unwrap_err();
        assert!(format!("{err}").contains("cannot read"), "{err}");

        let nowhere = Target {
            dir: file.path().join(".ssh/deeper"),
            owner: None,
        };
        let err = nowhere.append(&key("x")).unwrap_err();
        assert!(format!("{err}").contains("cannot create"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn what_is_made_for_a_user_goes_to_that_user() {
        use std::os::unix::fs::MetadataExt;
        let home = tempfile::tempdir().unwrap();
        let meta = fs::metadata(home.path()).unwrap();
        let target = Target {
            dir: home.path().join(".ssh"),
            owner: Some((meta.uid(), meta.gid())),
        };
        target.append(&key("one")).unwrap();
        target.append(&key("two")).unwrap();
        assert_eq!(fs::metadata(target.keys()).unwrap().uid(), meta.uid());
        assert_eq!(
            fs::read_to_string(target.keys()).unwrap(),
            format!("{}\n{}\n", key_text("one"), key_text("two"))
        );
        let stranger = Target {
            dir: target.dir.clone(),
            owner: Some((meta.uid() + 1, meta.gid())),
        };
        let err = stranger.append(&key("x")).unwrap_err();
        assert!(err.to_string().contains("not the user's own"), "{err}");
        let elsewhere = Target {
            dir: home.path().join("other"),
            owner: Some((meta.uid() + 1, meta.gid())),
        };
        let err = elsewhere.append(&key("x")).unwrap_err();
        assert!(err.to_string().contains("cannot hand over"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn a_directory_that_cannot_take_the_file_is_found_first() {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::tempdir().unwrap();
        let target = target(home.path());
        let locked = |path: &Path, mode| {
            fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
        };
        locked(home.path(), 0o500);
        let missing_dir = target.check(&mut Vec::new());
        locked(home.path(), 0o700);
        fs::create_dir(&target.dir).unwrap();
        locked(&target.dir, 0o500);
        let missing_file = target.check(&mut Vec::new());
        locked(&target.dir, 0o700);
        if writable_anyway(home.path()) {
            return;
        }
        for err in [missing_dir.unwrap_err(), missing_file.unwrap_err()] {
            assert!(err.to_string().contains("no write permission"), "{err}");
        }
    }

    #[cfg(unix)]
    fn writable_anyway(dir: &Path) -> bool {
        use std::os::unix::fs::PermissionsExt;
        let probe = dir.join("probe");
        fs::create_dir(&probe).unwrap();
        fs::set_permissions(&probe, fs::Permissions::from_mode(0o500)).unwrap();
        let root = writable(&probe);
        fs::set_permissions(&probe, fs::Permissions::from_mode(0o700)).unwrap();
        root
    }

    #[cfg(unix)]
    #[test]
    fn for_a_user_no_link_is_followed() {
        use std::os::unix::fs::{MetadataExt, symlink};
        let home = tempfile::tempdir().unwrap();
        let meta = fs::metadata(home.path()).unwrap();
        let owner = Some((meta.uid(), meta.gid()));
        let victim = home.path().join("victim");
        let refused = |dir: PathBuf| {
            let target = Target { dir, owner };
            format!("{:#}", target.append(&key("x")).unwrap_err())
        };

        let ssh = home.path().join("a");
        fs::create_dir(&ssh).unwrap();
        symlink(&victim, ssh.join("authorized_keys")).unwrap();
        assert!(refused(ssh.clone()).contains("without following a link"));
        assert!(!victim.exists());

        fs::write(&victim, "kept\n").unwrap();
        assert!(refused(ssh).contains("without following a link"));
        let linked = home.path().join("b");
        symlink(home.path().join("c"), &linked).unwrap();
        fs::create_dir(home.path().join("c")).unwrap();
        assert!(refused(linked).contains("without following a link"));

        let named_twice = home.path().join("d");
        fs::create_dir(&named_twice).unwrap();
        fs::hard_link(&victim, named_twice.join("authorized_keys")).unwrap();
        assert!(refused(named_twice).contains("other names"));
        assert_eq!(fs::read_to_string(&victim).unwrap(), "kept\n");
        assert_eq!(fs::metadata(&victim).unwrap().nlink(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn for_a_user_a_fifo_is_refused_without_waiting() {
        use std::os::unix::fs::{MetadataExt, symlink};
        use std::sync::mpsc;
        use std::time::Duration;
        let home = tempfile::tempdir().unwrap();
        let meta = fs::metadata(home.path()).unwrap();
        let owner = Some((meta.uid(), meta.gid()));
        let direct = home.path().join("direct");
        fs::create_dir(&direct).unwrap();
        let fifo = direct.join("authorized_keys");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success());
        let planted = home.path().join("planted");
        fs::create_dir(&planted).unwrap();
        symlink(&fifo, planted.join("authorized_keys")).unwrap();

        let (done, finished) = mpsc::channel();
        std::thread::spawn(move || {
            for dir in [planted, direct] {
                let target = Target { dir, owner };
                let checked = target.check(&mut Vec::new());
                let appended = target.append(&key("x"));
                done.send((checked.is_err(), appended.is_err())).unwrap();
            }
        });
        for _ in 0..2 {
            let outcome = finished.recv_timeout(Duration::from_secs(30)).unwrap();
            assert_eq!(outcome, (true, true));
        }
    }

    #[test]
    fn a_user_needs_an_absolute_home() {
        if cfg!(unix) {
            let me = Target::of_account("me", Path::new("/home/me"), 1000, 100).unwrap();
            assert_eq!(me.keys(), Path::new("/home/me/.ssh/authorized_keys"));
            assert_eq!(me.owner, Some((1000, 100)));
        }
        let err = Target::of_account("rel", Path::new("home/rel"), 5, 5).unwrap_err();
        assert!(err.to_string().contains("no absolute home"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn a_user_is_looked_up_on_the_system() {
        let root = Target::of(Some("root"), None).unwrap();
        assert_eq!(root.owner.map(|(uid, _)| uid), Some(0));
        let err = Target::of(Some("no-such-user-here"), None).unwrap_err();
        assert!(err.to_string().contains("no-such-user-here"), "{err}");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_account_gives_home_uid_and_gid() {
        let passwd = fs::read_to_string("/etc/passwd").unwrap();
        let entries = passwd.lines().filter_map(|line| {
            let fields: Vec<&str> = line.split(':').collect();
            let [name, _, uid, gid, _, home, ..] = fields.as_slice() else {
                return None;
            };
            let (uid, gid) = (uid.parse().ok()?, gid.parse().ok()?);
            Some((name.to_string(), (PathBuf::from(home), uid, gid)))
        });
        for (name, expected) in entries {
            assert_eq!(account(&name).unwrap(), Some(expected));
        }
    }

    #[cfg(not(unix))]
    #[test]
    fn a_user_is_refused_off_unix() {
        let err = Target::of(Some("me"), None).unwrap_err();
        assert!(err.to_string().contains("Unix only"), "{err}");
    }

    #[test]
    fn without_a_user_the_target_is_home() {
        let at = Target::of(None, Some(PathBuf::from("h"))).unwrap();
        assert_eq!(at.keys(), Path::new("h/.ssh/authorized_keys"));
        assert!(at.owner.is_none());
        assert!(Target::of(None, None).is_err());
    }

    #[test]
    fn one_key_in_ssh_goes_without_a_choice() {
        let home = tempfile::tempdir().unwrap();
        let ssh = home.path().join(".ssh");
        fs::create_dir(&ssh).unwrap();
        fs::write(ssh.join("id_ed25519"), "private").unwrap();
        fs::write(ssh.join("id_ed25519.pub"), key_text("me")).unwrap();
        let (found, said) = choose(home.path(), None, true, "");
        assert_eq!(found.unwrap(), key("me"));
        assert_eq!(said, "Sending public key type='ssh-ed25519' keyid='me'\n");
    }

    #[test]
    fn several_keys_are_offered_until_one_is_named() {
        let home = tempfile::tempdir().unwrap();
        fs::write(home.path().join("a.pub"), key_text("a")).unwrap();
        fs::write(home.path().join("b.pub"), key_text("b")).unwrap();
        let (found, said) = choose(home.path(), Some(home.path()), true, "9\nwhat\n 1 \n");
        assert_eq!(found.unwrap(), key("b"));
        assert!(said.contains("  0: a.pub\n  1: b.pub\n"), "{said}");
        assert_eq!(said.matches("Send which one?").count(), 3, "{said}");
        let (found, _) = choose(home.path(), Some(home.path()), true, "");
        assert_eq!(found.unwrap_err().to_string(), "no key was chosen");
    }

    #[test]
    fn the_key_is_sent_only_on_a_yes() {
        let home = tempfile::tempdir().unwrap();
        let file = home.path().join("k.pub");
        fs::write(&file, key_text("me")).unwrap();
        for yes in ["y\n", "YES\n"] {
            let (found, said) = choose(home.path(), Some(&file), false, yes);
            assert_eq!(found.unwrap(), key("me"));
            assert!(
                said.contains("Really send public key 'me' ? [y/N]: "),
                "{said}"
            );
        }
        for no in ["", "\n", "n\n", "sure\n"] {
            let (found, _) = choose(home.path(), Some(&file), false, no);
            assert_eq!(found.unwrap_err().to_string(), "the key was not sent");
        }
    }

    #[test]
    fn a_file_that_is_no_public_key_never_leaves() {
        let home = tempfile::tempdir().unwrap();
        let private = home.path().join("id_ed25519");
        fs::write(&private, "-----BEGIN OPENSSH PRIVATE KEY-----\nsecret\n").unwrap();
        let (found, said) = choose(home.path(), Some(&private), true, "");
        let err = format!("{:#}", found.unwrap_err());
        assert!(
            err.contains("cannot send") && !err.contains("secret"),
            "{err}"
        );
        assert_eq!(said, "");
        let (found, _) = choose(home.path(), Some(&home.path().join("gone")), true, "");
        assert!(found.unwrap_err().to_string().contains("cannot read"));
    }

    #[test]
    fn no_keys_is_an_error_naming_where_it_looked() {
        let home = tempfile::tempdir().unwrap();
        let (found, _) = choose(home.path(), None, true, "");
        assert!(found.unwrap_err().to_string().contains("cannot read"));
        fs::create_dir(home.path().join(".ssh")).unwrap();
        fs::create_dir(home.path().join(".ssh/dir.pub")).unwrap();
        let (found, _) = choose(home.path(), None, true, "");
        assert!(found.unwrap_err().to_string().contains("no public keys"));
        let none = chosen_key(None, true, None, &mut &b""[..], &mut Vec::new()).unwrap_err();
        assert!(none.to_string().contains("HOME"));
    }
}
