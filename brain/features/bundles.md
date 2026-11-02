# Bundles: several paths, kept paths

```
wyrm send a.txt photos/              # two paths: one bundle
wyrm send -R /etc/x/y.conf           # a kept path: a bundle, even alone
wyrm receive CODE                    # lands in the current directory
wyrm receive -o / CODE               # lands under /, so at /etc/x/y.conf
```

One path without `-R` travels as before: a file ([files.md](files.md)) or a
directory ([directories.md](directories.md)), under its own name. Several
paths, or any path given with `-R` (`--relative`), travel as one **bundle**: a
set of paths that each land at their name under the receiver's destination,
side by side, with no directory around them.

## What each path is named

Without `-R` a path is named by its last component, taken after resolving the
path, so `wyrm send ..` sends the parent under its real name. With `-R` it
keeps the path as typed, the way `rsync -R` and `tar` do: the leading `/` (and
on Windows the drive) is dropped, `.` components are dropped, and a `/./` marks
where the kept part starts; on Windows `\.\` marks it too. A trailing slash
changes nothing, unlike rsync, and `..` cannot be kept.

| Sent | Lands as |
| --- | --- |
| `wyrm send a.txt photos/` | `a.txt`, `photos/...` |
| `wyrm send docs/a.txt b.txt` | `a.txt`, `b.txt` |
| `wyrm send -R docs/a.txt` | `docs/a.txt` |
| `wyrm send -R /etc/x/y.conf` | `etc/x/y.conf` |
| `wyrm send -R /etc/./x/y.conf` | `x/y.conf` |
| `wyrm send -R photos` or `-R photos/` | `photos/...` |
| `wyrm send -R photos/./` | what is inside `photos`, without `photos/` |
| `wyrm send -R ../a.txt` | refused: `..` cannot be kept |
| `wyrm send -R .././a.txt` | `a.txt` |
| `wyrm send a/x.txt b/x.txt` | refused: two of the paths are named `x.txt` |
| `wyrm send -R etc etc/x/y.conf` | refused: `etc/x/y.conf` is inside `etc`, which is sent too |

Symlinks are followed when the files are read, as for a directory, and never
resolved in the kept name. `-R` is an option of `send`, so it needs the
explicit form (`wyrm send -R ...`), not the bare one.

## Where it lands

| Receiver runs | The bundle lands in |
| --- | --- |
| `wyrm receive CODE` | the current directory |
| `wyrm receive -o DIR CODE` | `DIR`, created with its parents when missing; refused when `DIR` is a file |

The prompt reads `Receiving 3 files (12 bytes) into: DIR`. Inside the
destination the bundle merges:

| At a path the bundle brings | Without `--overwrite` | With `--overwrite` |
| --- | --- | --- |
| nothing | created, with any missing directories | the same |
| a directory, where the bundle has a directory | used as it is | the same |
| a file, where the bundle has a file | the whole bundle is refused: `refusing to overwrite PATH` | replaced |
| a directory, where the bundle has a file | refused | refused: a directory is never replaced |
| a file, where the bundle has a directory | refused: `PATH is in the way` | refused |

Existing directories are used even when they are symlinks the receiver made.
When the bundle travels as a collection, every path is checked against the
destination as soon as the manifest arrives, before any file data; when it
travels as a zip, the check happens while it unpacks, after the download.

`merge::Merge` places each file the way a single file is placed: written as
`.<name>.<unique>.wyrm-part` next to where it goes, then hard-linked into
place, so a file that appeared there meanwhile is never overwritten (renamed
where hard links do not work, and renamed over the old file under
`--overwrite`). A transfer that fails on the way removes the files and
directories it created, leaves what was there before, and keeps the new
content of a file it already replaced under `--overwrite`.

## On the wire

A bundle is offered under the name `files`, with a `bundle` flag:

```json
{"wyrmyon-collection-v1": {"name": "files", "numbytes": 12, "numfiles": 3, "bundle": true}}
```

| Receiver | What the sender offers | Where it lands |
| --- | --- | --- |
| a wyrm, over iroh | a collection with `"bundle": true` | loose in the destination, as above |
| a wyrm, over classic transit (the sender ran `--force-classic`) | a zip directory offer with `"bundle": true` | loose in the destination, as above |
| a wyrm that runs `--force-classic` itself | a plain zip directory offer: it advertises nothing, so it is a legacy client | in a directory named `files` |
| a wyrmyon 0.4.0 or older | the same offers; it ignores the flag | in a directory named `files` |
| the Python client, or any legacy client | a plain zip directory offer, no flag | in a directory named `files` |

The flag goes into a zip offer only when the peer's encrypted version message
advertises `iroh-v1`, so a legacy client never sees it
([negotiation.md](../architecture/negotiation.md)). The zip itself is received
as `.wyrm-bundle.<unique>.wyrm-part` in the destination. Under the `wormhole`
name a bundle lands loose too: it only comes from a wyrm.

## Sources

- [crates/cli/src/send.rs](../../crates/cli/src/send.rs) (`kept`, `paths_payload`, `send_tree`)
- [crates/cli/src/zipdir.rs](../../crates/cli/src/zipdir.rs) (`list_bundle`, `extract_with`)
- [crates/cli/src/merge.rs](../../crates/cli/src/merge.rs)
- [crates/cli/src/receive.rs](../../crates/cli/src/receive.rs) (`bundle_base`, `receive_directory`, `receive_collection`)
- [crates/cli/src/protocol.rs](../../crates/cli/src/protocol.rs)
- [crates/transport-iroh/src/collection.rs](../../crates/transport-iroh/src/collection.rs) (`Placement`, `Under`)
