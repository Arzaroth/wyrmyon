# Verified, resumable transfers

Between two wyrms (iroh-v1), a file travels as an iroh-blobs blob, and a
directory as a collection of blobs, one per file
([directories.md](directories.md)), or as one zip blob with an older wyrm: every 16 KiB chunk group is checked against a BLAKE3 tree
before it is written, and an interrupted transfer picks up where it stopped
the next time the same content is sent, whatever the code.

```
wyrm send big.iso        # Ctrl-C, a dropped network, a closed laptop...
wyrm send big.iso        # a new code; the receiver fetches only what it lacks
```

## How it works

- **Sender.** Once iroh is chosen, `Offered::import` hashes the file into a
  throwaway iroh-blobs store, referencing it in place rather than copying it
  (`Hashing..`). After the channel binding it writes the 32-byte BLAKE3 hash on
  the control stream and serves iroh-blobs requests arriving on further
  streams of the same, already authenticated connection; nothing else can
  reach that store. It then waits for the ack on the control stream.
- **Receiver.** Reads the hash, opens the store at
  `<cache>/<hash>/` and calls iroh-blobs' `fetch`, which requests only the
  ranges the store does not hold yet; the progress bar starts at what was
  already there. The fetch stops the moment more bytes arrive than were
  offered, and a completed blob whose size is not the offered size is
  refused: either way the data is wrong and its cache entry is deleted, so a
  peer cannot fill the cache with more than the user accepted. The blob is
  exported into the partial file next to the destination (moved rather than
  copied when the store owns the data on the same filesystem, except on
  Windows, which cannot move a file the store holds open; then fsynced),
  finished the usual way ([files.md](files.md),
  [directories.md](directories.md)), and acked with `{"ack": "ok"}`: no
  SHA-256, since BLAKE3 already verified every byte.
- **The cache.** `WYRMYON_CACHE_DIR`, else `$XDG_CACHE_HOME/wyrmyon/partial`,
  else `~/.cache/wyrmyon/partial`; on Windows `%LOCALAPPDATA%` and
  `%USERPROFILE%` stand in for the last two. Empty or relative values are
  ignored. A
  successful transfer, or one whose data turned out wrong, deletes its
  `<hash>` directory. An interrupted one keeps it for the next attempt and
  says so in the error, naming the directory to delete if no retry is
  coming. Exporting moves data out of the store, so a `<hash>.exporting`
  marker goes down first: an export that fails (a full disk, say) deletes
  the entry, and an entry found with its marker, left by a process killed
  mid-export, is started afresh rather than trusted. Nothing evicts abandoned
  entries: they stay until the same content arrives or the user removes them.
  One store per hash means resuming is just reopening it, and cleaning up is
  one directory.

A collection resumes the same way: its cache entry is named after the hash of
the whole collection, so sending the same files again picks up where the last
attempt stopped, the manifest and every file already verified included. Change
one file and the collection gets a new hash, so the next attempt starts from
nothing.

Classic transit with a legacy peer is unchanged: records, SHA-256, no resume.

The receiver pulls the data, so the sender's bar follows what the blob
provider reports sending ([cli.md](cli.md#progress)); a resumed blob starts
at the offset the receiver asked from.

## Sources

- [crates/transport-iroh/src/blobs.rs](../../crates/transport-iroh/src/blobs.rs)
- [crates/transport-iroh/src/collection.rs](../../crates/transport-iroh/src/collection.rs)
- [crates/cli/src/send.rs](../../crates/cli/src/send.rs)
- [crates/cli/src/receive.rs](../../crates/cli/src/receive.rs)
- [crates/cli/src/lib.rs](../../crates/cli/src/lib.rs) (`cache_dir`)
