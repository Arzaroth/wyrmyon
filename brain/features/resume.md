# Verified, resumable transfers

Between two wyrms (iroh-v1), a file or a directory's zip travels as an
iroh-blobs blob: every 16 KiB chunk group is checked against a BLAKE3 tree
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
  already there. A completed blob whose size is not the offered size is
  refused. The blob is exported into the partial file next to the
  destination, finished the usual way ([files.md](files.md),
  [directories.md](directories.md)), and acked with `{"ack": "ok"}`: no
  SHA-256, since BLAKE3 already verified every byte.
- **The cache.** `WYRMYON_CACHE_DIR`, else `$XDG_CACHE_HOME/wyrmyon/partial`,
  else `~/.cache/wyrmyon/partial`. A successful transfer, or one whose data
  turned out wrong, deletes its `<hash>` directory; an interrupted one leaves
  it for the next attempt. One store per hash means resuming is just
  reopening it, and cleaning up is one directory.

Classic transit with a legacy peer is unchanged: records, SHA-256, no resume.

## Sources

- [crates/transport-iroh/src/blobs.rs](../../crates/transport-iroh/src/blobs.rs)
- [crates/cli/src/send.rs](../../crates/cli/src/send.rs)
- [crates/cli/src/receive.rs](../../crates/cli/src/receive.rs)
- [crates/cli/src/lib.rs](../../crates/cli/src/lib.rs) (`cache_dir`)
