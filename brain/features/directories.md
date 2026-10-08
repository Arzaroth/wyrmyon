# Directories

```
wyrm send photos/
wyrm receive --accept-file CODE    # writes ./photos/
```

A directory travels as one zip file, the way the Python client sends it, so
it works both ways with any magic-wormhole client.

## Sender

`zipdir::build` walks the directory in name order into a temporary zip file
(`tempfile`, removed on exit): files deflated with their permission bits,
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
- refuses any entry whose path leaves the directory (absolute, `..`);
- skips symlink entries;
- creates every file fresh, with its permission bits masked to `0o777` (no
  setuid);
- stops with an error as soon as the files outnumber `numfiles` or their bytes
  exceed `numbytes`, so a small zip that inflates far past what was offered
  never fills the disk.

The ack carries the SHA-256 of the zip file as received.

Building and extracting run on blocking threads. Both check a cancel flag
between files and between reads, set when the transfer is dropped (Ctrl-C
included), so an interrupted transfer stops promptly and leaves no
half-extracted directory.

## Sources

- [crates/cli/src/zipdir.rs](../../crates/cli/src/zipdir.rs)
- [crates/cli/src/send.rs](../../crates/cli/src/send.rs)
- [crates/cli/src/receive.rs](../../crates/cli/src/receive.rs)
