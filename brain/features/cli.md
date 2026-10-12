# The command line

```
wyrm report.pdf photos/      # paths: send (several travel as one bundle)
echo "hello" | wyrm          # piped stdin: send it as text
wyrm                         # nothing, in a terminal: ask for a code, receive
wyrm 7-guitarist-revenge     # a code: receive
wyrm receive --new           # receiver first: allocate a code, wait

wyrm send ... / wyrm receive ...   # explicit forms, with every option
```

## Bare arguments

Without a subcommand, `bare()` in `lib.rs` picks one from what was given:

| Arguments | Action |
| --- | --- |
| none, stdin a terminal | `receive`, asking for the code |
| none, stdin piped | `send --text -` |
| one argument shaped like a code (`7-word-word`) | `receive CODE`, unless a file by that name exists here: then it refuses and asks for `send` or `receive` |
| anything else | `send PATHS` |

Global options (`--relay-url`, `--force-classic`, `--hide-progress`...) work in
either form; the subcommands' own options (`--accept-file`, `-o`, `--code`)
need the explicit form. A bare receive still asks before accepting a file.

## The code prompt

In a terminal, the code is read with rustyline and Tab completes the word
being typed from the PGP word list, odd and even words in turn as the code
uses them (`wyrmyon_wormhole::code::completions`), for as many words as
`--code-length`. Without a terminal it reads one line from stdin, so the code
can be piped in.

## The code and the hint

The side that allocates the code prints it with what the other side should
run: `wyrm CODE` (or `wormhole receive CODE`) after `send`, and
`wyrm send --code CODE FILE` after `receive --new`. `--qr` on either also
draws `wormhole-transfer:CODE`, the URI the Python client puts in its QR code,
for a phone app to scan. Like Python's, it is drawn in half blocks with the
light modules inked and a one-module border, which reads right on a dark
terminal; `--ascii` draws `##` instead. `qr::render` builds it from the
`qrcode` crate's module grid.

## Progress

Both sides draw a bar while the data moves, on stderr and only in a
terminal: a wyrm (`∿∿∿ᗧ· · ·`) eating its way to the end, its head chomping
on a steady tick so a stalled transfer still looks alive. `--ascii` (or
`WYRMYON_ASCII`) draws `~~~<. . .` instead, and `--hide-progress` draws
nothing. Over classic transit the sender counts the records it writes; over
iroh the receiver pulls, so the sender follows `IrohPipe::sent()`, the bytes
of the offered files the blob provider has sent, from its request events
(the manifest and the hash list are not counted).

## Several paths

`wyrm send a.txt photos/` (or bare `wyrm a.txt photos/`) offers the paths
side by side, each under its own name, as one directory named `files`: a
collection between two wyrms, a zip file any other receiver unpacks like a
directory ([directories.md](directories.md)). Two paths with the same name are
refused before anything is sent.

## Sources

- [crates/cli/src/lib.rs](../../crates/cli/src/lib.rs) (`Cli`, `bare`, `show_code`)
- [crates/cli/src/qr.rs](../../crates/cli/src/qr.rs)
- [crates/cli/src/transfer.rs](../../crates/cli/src/transfer.rs) (`progress`, `wyrm`, `follow_sent`)
- [crates/transport-iroh/src/blobs.rs](../../crates/transport-iroh/src/blobs.rs) (`count_sent`)
- [crates/cli/src/receive.rs](../../crates/cli/src/receive.rs) (`prompt_code`, `CodeCompleter`)
- [crates/cli/src/send.rs](../../crates/cli/src/send.rs) (`paths_payload`)
- [crates/cli/src/zipdir.rs](../../crates/cli/src/zipdir.rs) (`build_bundle`)
- [crates/wormhole-core/src/code.rs](../../crates/wormhole-core/src/code.rs)
