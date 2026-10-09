# Decisions

Each entry is a choice a reader might want to undo, with the reason not to.

## Our own protocol code, not magic-wormhole.rs

magic-wormhole.rs is EUPL-1.2, a copyleft licence; linking it would make the
shipped binary EUPL while wyrmyon is MIT. The client side of the protocol is
small (a few WebSocket messages, SPAKE2, HKDF, secretbox) and the Python client
is a precise reference, so `wormhole-core` implements it and the interop suite
holds it to the reference.

## One reader on the mailbox connection

The mailbox connection is read by whoever is waiting, never by a background
task. The protocol is a strict request/response dance with the peer's messages
in between, so buffering those messages while waiting for a server reply is
enough, and there is no task to cancel or channel to drain on close. The cost:
no reconnection after a dropped mailbox connection, which the Python client
does handle. Revisit if transfers fail on flaky networks.

## Close the mailbox on every failure

On the real server a dropped connection frees nothing: a claimed nameplate and
an open mailbox stay until pruning, 11 to 16 minutes later, on a shared
community server. So every failure after the claim releases and closes, with
the mood the Python client would send: `scary` when the peer's messages do not
check out (the server's signal of someone guessing codes), `errory` for server
errors, and `happy` for an application-level refusal, which is not an error of
the protocol.

## Refuse rather than ask without a terminal

The Python receiver asks before accepting a file and fails when stdin is not a
terminal, which leaves its sender waiting. `wyrm receive` refuses with an
`error` message instead, and `--accept-file` is the way to receive
unattended. Writing into a fresh `.<name>.wyrm-part` and renaming at the end
means a failed or hostile transfer never leaves a file that looks complete.

## Directories create their destination, and stop at what was offered

The receiver creates the destination directory itself (`create_dir`, which
fails if anything is there) instead of checking and then extracting, so it
never merges into a directory that appeared meanwhile, and removes it if
extraction fails. Extraction counts files and bytes against the offer and
stops past either: the offer is what the user said yes to, and a zip's own
headers can claim anything. Symlink entries are skipped on both sides, as the
Python receiver never recreates them either.

## No fallback from iroh to classic transit

When both peers chose iroh-v1 and iroh cannot connect, the transfer fails
with a message, rather than falling back to classic transit on the same
wormhole. A fallback needs both sides to agree when to give up and what to
try next, a second negotiation that can itself go wrong; iroh already falls
back to its relays, which reach almost anywhere. `--force-classic` is the
manual way out.

## Our own wire form for iroh addresses

`IrohInfo` (`id`, `relays`, `direct`) is defined here rather than serialising
iroh's `EndpointAddr`, whose serde form is iroh's to change between releases.
The endpoint uses the `Minimal` preset: no pkarr or DNS publishing, since the
address only ever needs to reach the one peer, through the encrypted mailbox.

## Close iroh endpoints, never drop them

Dropping an iroh endpoint does not send the QUIC close: the peer only notices
after the 30 s idle timeout, and an ack still in flight is lost. Every path
out of `IrohTransport::connect` and `IrohPipe::finish` closes the endpoint, and
the side that writes the ack waits for the other's close first.

## iroh-blobs inside the bound connection, one store per hash

The receiver fetches with iroh-blobs over the very connection that passed the
node ID pin and the channel binding: the sender serves blob requests on that
connection's streams and has no `iroh-blobs` ALPN on its endpoint, so nobody
else can ask for anything. The hash travels on the bound control stream, not
the mailbox, so the mailbox protocol did not change. The sender imports by
reference, so a large file is hashed but not copied. The receiver keeps one
store per hash under the cache directory: resuming is reopening it, a
finished transfer or bad data removes one directory, and an interrupted one
leaves it with a message saying where. Nothing evicts abandoned entries; that
is the price of resuming across codes. Two receivers fetching the same content
on one machine at once would contend for the same store; the second fails
rather than corrupting it. Exporting replaces the placeholder partial file by
path, which a local user able to write the destination directory could race
with a symlink; such a user can already write there, so this is left as is.

## Interoperate first, upgrade second

wyrmyon is a magic-wormhole client before it is anything else. A tool that
only talks to itself needs both people to install it; one that talks to the
existing network is useful from the first transfer, and the iroh path is a
bonus when both ends happen to have it.

## Choose the transport after PAKE, not before

The capability advert rides in the encrypted version message. Deciding earlier
(a nameplate prefix, a mailbox side message) would either show legacy clients
something they do not understand or let the mailbox server tamper with the
choice. After PAKE the advert is authenticated by the code, and both peers
decide from the same two messages at once, so there is no race and no
downgrade.

## Always the public mailbox

The sender cannot know which client the receiver will run, so the code has to
live where every client looks. Our own mailbox is only a fallback when the
public one is unreachable, and its codes are marked so a legacy client refuses
them rather than waiting on the wrong server.

## Pin the node ID and bind the channel

The NodeAddr comes through the encrypted mailbox, so pinning its node ID makes
QUIC/TLS reach exactly the endpoint the peer announced. The MAC exchange keyed
from the wormhole key then proves that endpoint belongs to whoever knows the
code. Either check alone leaves a gap; see
[architecture/iroh-v1.md](architecture/iroh-v1.md).

## Legacy path first

Classic transit is built before iroh-v1 (M1 to M3 before M4) because the Python
CLI tests it for free and it is what most transfers will use. The iroh path then
lands as a pure upgrade behind the same `Transport` trait.

## The name: wyrmyon, installed as `wyrmyon` and `wyrm`

A wyrm (a wormhole pun) made of tachyon-like particles: the `-yon` is the
particle ending of tachyon, luxon and tardyon. The name was free on crates.io,
PyPI, npm and Homebrew with no GitHub repository using it. `wyrm` is shorter to
type but taken as a package name on crates.io and PyPI (libraries, without a
`wyrm` binary), so it ships only as a second binary inside the `wyrmyon`
package. Internal crates carry a `wyrmyon-` prefix for the same reason.

## Wheels for distribution

maturin's `bindings = "bin"` puts the native binary in a wheel, so `uvx` and
`pipx` users (the people who already have the Python `wormhole`) get it with no
Python runtime involved, as ruff and uv do.

## Binaries, not crates

wyrmyon ships as release archives and packages (PyPI wheels and a Homebrew
formula later); no crate goes to crates.io (`publish = false` everywhere). The crates
are internals of one tool, not a library with a stable API, and publishing
them would freeze names and versions nobody asked for; `cargo install --git`
still works for Rust users. The workspace crates depend on each other by path
only, so the version lives in one line of the workspace `Cargo.toml`.

## Our own release workflow, static musl on Linux

The release workflow is ours rather than cargo-dist's: cargo-dist builds
archives, shell and PowerShell installers and MSIs, but no .deb, .rpm or Arch
package, and bolting those onto its generated workflow left two pipelines to
reason about. nfpm builds the .deb and .rpm from one config with `wyrm` as a
real symlink, which cargo-generate-rpm cannot do; makepkg builds the Arch
packages from the PKGBUILDs the AUR will get, so they are tested by the same
run. The Linux binaries are static musl: one binary per architecture for
every package format and distribution, with no glibc floor, as fd and atuin
ship. Every package is installed on a matching system before the release
job, which runs last, so a broken package never reaches a published release.
