# Directories

```
wyrm send photos/
wyrm receive --accept-file CODE    # writes ./photos/
```

A directory travels as one zip file, the way the Python client sends it, so
it works both ways with any magic-wormhole client. Between two wyrms that both
advertise `collection-v1` it travels as a collection instead: one blob per
file, no zip ([Between two wyrms](#between-two-wyrms-a-collection)).

## Sender

`zipdir::list` walks the directory in name order into a list of entries, before
the code is shown; once the peer is known, `zipdir::zip` writes that list into a
temporary zip file (`tempfile`, removed on exit), or the list becomes a
collection. In the zip: files deflated with their permission bits
(`0o644`, or `0o444` when read-only, on Windows),
empty directories as directory entries (an empty top directory as `./`, which
the Python receiver needs to create anything), symlinks followed as the Python
client does, except one leading back to a directory already being walked,
anything that is not a file or directory skipped. Files from 3.75 GiB up get
zip64 headers. The name offered is the last component of the resolved path, so
`wyrm send ..` sends the parent under its real name. The offer is

```json
{"directory": {"mode": "zipfile/deflated", "dirname": "photos",
               "zipsize": 1234, "numbytes": 5678, "numfiles": 3}}
```

and the zip file then streams exactly like a file ([files.md](files.md)),
answered by the same `file_ack`.

## Receiver

The same checks as a file before anything is accepted: a usable name, a
transit offer, nothing at the destination, confirmation or `--accept-file`.
An unknown `mode` is refused. The zip file streams into a partial file next to
the destination, then `zipdir::extract`:

- creates the destination directory itself, so it fails rather than merge
  into one that appeared meanwhile, and removes it again if extraction fails;
- refuses any entry whose path leaves the directory (absolute, `..`), and on
  Windows any component it cannot hold, the names a single file refuses
  ([files.md](files.md)): a drive-relative `c:x` in the middle of a path
  would otherwise land outside it;
- skips symlink entries;
- creates every file fresh, with its permission bits masked to `0o777` (no
  setuid) on Unix, with the platform's defaults elsewhere;
- stops with an error as soon as the files outnumber `numfiles` or their bytes
  exceed `numbytes`, so a small zip that inflates far past what was offered
  never fills the disk.

The ack carries the SHA-256 of the zip file as received.

## Between two wyrms: a collection

When iroh-v1 is chosen and the peer's version message lists `collection-v1`
under `offers` ([negotiation.md](../architecture/negotiation.md)), there is no
zip. The sender imports every file of the list into its throwaway store by
reference, hashing it in place, and offers

```json
{"wyrmyon-collection-v1": {"name": "photos", "numbytes": 5678, "numfiles": 3}}
```

The data is an iroh-blobs hash sequence: the first child is our manifest, a
JSON list of every file's `/`-separated path, size and permission bits, plus
the empty directories; the other children are the files, in the manifest's
order.

```json
{"files": [{"path": "sub/b.bin", "size": 100, "mode": 420}], "dirs": ["empty"]}
```

The receiver asks the same question as for a zipped directory, creates the
destination itself (failing if anything is there), and then:

- fetches only the hash sequence and the manifest, at most
  `32 × (numfiles + 1)` bytes plus 16 MiB, and checks the manifest before any
  file data: as many files as offered (at most 2^20, refused before
  accepting otherwise), sizes adding up to the offered bytes, at most 65536
  empty directories, no path over 4096 bytes, no empty, `.` or `..` segment
  and no backslash, no path twice, no path both a file and a directory's
  parent (walked as a tree of components, so a hostile path costs linear
  time), and every path holdable here by the same rules as a zip entry
  (nothing absolute, nothing Windows cannot hold);
- fetches the files, stopping the moment more bytes arrive than were offered,
  and checks every file is complete at its listed size;
- exports each file into place (moved out of the store where it can, copied
  for a second file with the same content and on Windows), with its permission
  bits masked to `0o777` on Unix, and creates the empty directories.

A manifest or data that does not check out deletes the cache entry and the
destination; an interrupted transfer keeps the cache entry for the next
attempt, and a failed export deletes it ([resume.md](resume.md)). Once every
file is exported the directory is kept, even if the ack then fails. Symlinks are followed on the sender as for a
zip, and never created on the receiver.

## Cancelling

Building and extracting run on blocking threads. Both check a cancel flag
between files and between reads, set when the transfer is dropped (Ctrl-C
included), so an interrupted transfer stops promptly and leaves no
half-extracted directory. A read after the flag is set fails with a plain
I/O error: `io::copy` retries an `Interrupted` one forever.

## Sources

- [crates/cli/src/zipdir.rs](../../crates/cli/src/zipdir.rs)
- [crates/transport-iroh/src/collection.rs](../../crates/transport-iroh/src/collection.rs)
- [crates/cli/src/send.rs](../../crates/cli/src/send.rs)
- [crates/cli/src/receive.rs](../../crates/cli/src/receive.rs)
