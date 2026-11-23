# The transfer library

`crates/transfer` (package `wyrmyon-transfer`) holds what a send or a receive
does once the wormhole is paired: the app messages, choosing and opening the
transport, the pipe, hashing, the offer and its answer, the data and the ack.
It prints nothing and owns no file layout, so the CLI and the Android app
([features/index.md](../features/index.md), M11 in the roadmap) drive the
same code. The CLI keeps what is about a terminal and a filesystem: clap,
progress bars, prompts, destinations, zips, merges and partial files.

## API

| Item | What it does |
| --- | --- |
| `protocol` | The app messages in the Python client's shapes (`Offer`, `Answer`, `AppMessage`), and `send`, `next`, `send_error` on a wormhole |
| `Options` | The settings a transfer needs: mailbox URL, transit helper, `no_listen`, `force_classic`, `force_iroh`, iroh relays. `config()` is the wormhole `Config` with our `app_versions` advert ([negotiation.md](negotiation.md)); `use_iroh` and `use_collection` read the peer's; `transit` and `iroh_or_tell` open either transport |
| `Pipe` | Either transport's connected pipe; `send_stream` / `receive_stream` move a stream over classic records with a SHA-256; `await_ack` / `send_ack` end a transfer |
| `Source` | What a sender reads: a size, `open()` for a fresh reader each time (one for hashing, one per classic receiver), and an optional `path()`. `FileSource` is a file on disk |
| `Hashing`, `Staged` | The iroh blob or collection imported ahead of the offer, kept for every receiver of a `--times` run ([features/several-receivers.md](../features/several-receivers.md)) |
| `send_data` / `send_over` / `offer_text` | Send a file (or any source) with its offer, over iroh when the peer can, otherwise classic transit; the text offer |
| `await_offer` | Reads the sender's messages up to the offer, answering its `transit` or `wyrmyon-iroh-v1` with ours on the way, and returns the offer with the `Connection` |
| `receive_into(..., Sink, ...)` | Sends the `file_ack`, connects and receives into a `Sink`, then `Received::complete` acks once the caller has put the data in place (or aborts) |
| `Tree`, `send_tree` / `receive_tree` | Several files and folders from sources, sent as a collection or the legacy zip, received into a writer per file; `usable_path` is the check a received path passes |
| `Events` | What a transfer reports: hashing, connected (with the pipe), bytes moved, settle, awaiting the ack, stopped. `cancelled()` is the other way: a future that, once it resolves, stops the hashing, the wait for the receiver's answer or the data with `Cancelled`, closing the pipe (the sender tells a receiver still waiting for the offer `the sender cancelled`); by default it never does. `Quiet` reports nothing; the CLI's `transfer::terminal` draws bars and prints the lines the brain's feature docs quote |

## Streams, and the copies left

A `Source` with a `path()` is imported by reference into the iroh blob store,
as before. Without one, `Offered::import_reader` streams it in: iroh needs
the BLAKE3 hash before the offer and serves verified ranges after it, so the
data is copied once into the transfer's throwaway store. Classic transit
reads the source as it sends. A source that ends early or runs long fails
the send, as a file that shrinks or grows does.

A `Sink` is `File { file, path }` or any `Writer`. Classic transit writes
into it as records arrive. iroh fetches into the cache first (that is what
makes it resumable, [features/resume.md](../features/resume.md)), then
exports by reference or rename to a path, or copies into a writer with
`Fetched::export_writer`; a writer that fails keeps the cache for the next
attempt.

iroh-blobs' `add_stream` never finishes when its stream yields an error, so
`add_reader` ends the stream early instead and reports the read error, or a
source longer than its size, once the import returns.

A `Tree` (`tree.rs`) is several sources with their relative paths, empty
folders, and whether it is a bundle. `send_tree` imports it as an iroh
collection when the peer offers `collection-v1`
(`Offered::import_streams`), otherwise zips it into a scratch file (deflated,
zip64 from 3.75 GiB, as the CLI does) and sends that as the legacy directory
offer; a source that ends early or runs long fails the zip.
`receive_tree` takes either back into a writer per file, from a callback
given each relative path: a collection is fetched then copied out of the
cache (`Collection::export_each`); a zip is received into the cache and
unpacked there, its entries checked as the CLI checks them (inside the
directory, plain relative paths, no more files or bytes than offered).
Empty folders reach the callback only through the paths of the files under
them. The CLI keeps its path-based zip and export, which merge into existing
folders and set modes.

## Sources

- [crates/transfer/src/lib.rs](../../crates/transfer/src/lib.rs)
- [crates/transfer/src/protocol.rs](../../crates/transfer/src/protocol.rs)
- [crates/transfer/src/options.rs](../../crates/transfer/src/options.rs)
- [crates/transfer/src/pipe.rs](../../crates/transfer/src/pipe.rs)
- [crates/transfer/src/source.rs](../../crates/transfer/src/source.rs)
- [crates/transfer/src/send.rs](../../crates/transfer/src/send.rs)
- [crates/transfer/src/receive.rs](../../crates/transfer/src/receive.rs)
- [crates/transfer/src/events.rs](../../crates/transfer/src/events.rs)
- [crates/transfer/src/tree.rs](../../crates/transfer/src/tree.rs)
- [crates/transport-iroh/src/blobs.rs](../../crates/transport-iroh/src/blobs.rs) (`add_reader`, `export_writer`)
- [crates/transport-iroh/src/collection.rs](../../crates/transport-iroh/src/collection.rs) (`import_streams`, `export_each`)
- [crates/cli/src/transfer.rs](../../crates/cli/src/transfer.rs) (`terminal`)
