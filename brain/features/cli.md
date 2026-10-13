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
`wyrm send --code CODE FILE` after `receive --new`. `wyrm send --qr` also
draws `wormhole-transfer:CODE`, the URI the Python client puts in its QR code,
for a phone app to scan; with `--code` it draws the code given. Like
Python's, it is drawn in half blocks with a one-module border and, on a
terminal, in fixed colours (white on near-black), so it scans whatever the
theme; `--ascii` draws `##` instead, which reads right on a dark terminal.
It is encoded at error correction level H (about 30% recoverable) and a
pixel-art drake facing right sits in the middle, on a cleared rectangle with a
one-module dark margin: 7x5 modules for versions 3 and 4, 11x7 for 5 to 7,
15x9 from 8 up, the largest whose cleared rectangle stays within 9% of the
modules. It never covers a finder, timing, format or version module, nor an
alignment pattern: versions 7 to 13, 21 to 27 and 35 to 40 hold one at the
centre, so there the drake moves to the nearest spot clear of it (4 to 8
modules up or down). A code too long for level H (over 1255 characters)
falls back to the `qrcode` default level
with no drake. On a terminal the drake is drawn in an aurora (violet, blue,
cyan, mint, tail to head) in 256-colour codes, and a character holding a
drake module sets both halves' colours; without colour it is drawn as light
modules. `qr::grid` builds the module grid, drake included, from the
`qrcode` crate's; `qr::render` draws it. There is no
`--qr` on `receive --new`: the URI's default role tells a scanning app to
receive, and the Python client has none there either. The URI carries no
`rendezvous`, as Python's does not, so a phone meets on its default mailbox.

## Progress

Both sides draw a bar while the data moves, on stderr and only in a
terminal: a wyrm (`wyrm.rs`) drawn in braille, so each character holds two
by four dots. Its body is a wave that runs from the head back to the tail,
so every part of it rises and falls, with an aurora gradient (violet to
mint) and a highlight running along it; its head is a fixed dragon sprite
eight dots wide, facing right, with a mane spike and a jaw that chomps.
The motion carries the transfer's state:

- **Speed.** The wave and the chomping go faster with the rate (smoothed
  from the position, on a log scale from 64 KiB/s to 16 MiB/s).
- **Stall.** After 1.2 s without a byte, or once done, it rests: the body
  keeps most of its curve, drifts slowly and breathes, and the mouth stays
  shut.
- **Fire.** When the rate more than doubles past 1 MiB/s once the wyrm has
  been moving (at most once in 3 s; say when hole-punching goes direct, or
  when a stalled transfer resumes; not at the start) and once at the end, it
  breathes fire for about a second: a flickering cone from the mouth over
  the bar ahead, white to dark red. A shown bar stays 0.8 s at the end
  (`transfer::finale`) so the last fire is seen; a hidden bar, or one off a
  terminal, goes at once.
- **Wormhole.** Over classic transit (another magic-wormhole client, or a
  wyrm told `--force-classic`), two spinning vortices (a braille spinner,
  one cell wide whatever the locale) sit at 25% and 75% of the bar and the
  wyrm is hollow, a dotted outline, between them. Over iroh there is none.

It redraws every 50 ms. Colour is 24-bit when `COLORTERM` says so (or under
Windows Terminal), 256 colours otherwise, and none under `NO_COLOR`,
`TERM=dumb` or off a terminal. A Windows console outside Windows Terminal
(no `WT_SESSION`) gets the ASCII wyrm without colour, as its fonts lack
braille and it may not take colour codes. `--ascii` (or `WYRMYON_ASCII`, any of `1`,
`true`, `yes`, `on`) draws it without colour in ASCII (`~-_'~@<*+:`,
the vortices as `O`), and `--hide-progress` draws nothing. A failed transfer
clears its bar before it aborts. Over classic transit the sender counts the records it writes; over
iroh the receiver pulls, so the sender follows `IrohPipe::sent()`, the bytes
of the offered files the blob provider has sent, from its request events
(the manifest and the hash list are not counted). A blob is credited in
full when the next one starts or the request completes, since the provider
drops progress events when they come faster than they are read.

## Several paths

`wyrm send a.txt photos/` (or bare `wyrm a.txt photos/`) offers the paths
side by side, each under its own name, as one directory named `files`: a
collection between two wyrms, a zip file any other receiver unpacks like a
directory ([directories.md](directories.md)). Two paths with the same name are
refused before anything is sent.

## Sources

- [crates/cli/src/lib.rs](../../crates/cli/src/lib.rs) (`Cli`, `bare`, `show_code`)
- [crates/cli/src/qr.rs](../../crates/cli/src/qr.rs)
- [crates/cli/src/transfer.rs](../../crates/cli/src/transfer.rs) (`progress`, `follow_sent`)
- [crates/cli/src/wyrm.rs](../../crates/cli/src/wyrm.rs)
- [crates/transport-iroh/src/blobs.rs](../../crates/transport-iroh/src/blobs.rs) (`count_sent`)
- [crates/cli/src/receive.rs](../../crates/cli/src/receive.rs) (`prompt_code`, `CodeCompleter`)
- [crates/cli/src/send.rs](../../crates/cli/src/send.rs) (`paths_payload`)
- [crates/cli/src/zipdir.rs](../../crates/cli/src/zipdir.rs) (`build_bundle`)
- [crates/wormhole-core/src/code.rs](../../crates/wormhole-core/src/code.rs)
