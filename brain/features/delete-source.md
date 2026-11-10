# Deleting what was sent

```
wyrm send --delete-source build.tar     # gone once the receiver confirms
wyrm send --delete-source --times 3 d/  # gone once all three confirm
```

`wyrm send --delete-source` deletes the paths it was given after the
transfer. It is a sender-side flag: nothing changes on the wire and the
receiver can be any client. `wormhole send` (the shim) does not take it, since
the Python client has no such option. It cannot be combined with `--text`.

## When it deletes

Only when the whole run succeeds: the receiver acked a verified copy (the
classic transit hash, or the BLAKE3-checked iroh blob or collection). With
`--times N` that means every one of the N receivers. A refused offer, a
failed or cancelled transfer, or one failed receiver out of N deletes nothing,
and the sender says `Nothing deleted: ...` before the error. Ctrl-C ends the
process before the delete step.

## What it deletes

Each path as typed, after dropping a trailing `/` or `/.`: with `-R`, the
whole typed path goes, not only the kept part. A directory goes with
everything in it. A link goes, never what it points at, even when the send
followed it (`link/`, or a directory link inside a tree).

When the run starts, before connecting, it walks each path without following
links and records every entry: files by size and modification time, links by
target, directories by their names. At the end it walks again, and a path
whose record differs (a file written, added or removed) is kept with
`Kept PATH: it changed during the transfer`, so it never deletes data the
receiver did not get. Each deleted path prints `Deleted PATH`. A path kept or
not deletable makes the run fail with `the transfer went through, but K of N
paths were not deleted`, so a script notices.

## What it refuses

Before connecting, it refuses a path that is or holds the current directory
or the home directory: `.`, `..`, `/`, `~`, and their parents. A link is
located by its own name, not its target, so a link to `~` can go. A missing
path is refused as a plain send would refuse it.

## Why no confirmation flag

A second, ceph-style `--yes-i-really-mean-it` flag guards a loss with no copy
left. Here the delete only follows a confirmed, verified copy on the other
side. The guard is the announcement instead: next to the code, before
anything moves, the sender prints `Once the receiver confirms, this deletes:
PATHS` (`Once all N receivers confirm` with `--times`), so a wrong path shows
while Ctrl-C still keeps everything.

## Sources

- [crates/cli/src/discard.rs](../../crates/cli/src/discard.rs)
- [crates/cli/src/send.rs](../../crates/cli/src/send.rs)
- [crates/cli/tests/cli.rs](../../crates/cli/tests/cli.rs)
