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

The sender prints the code with what the other side should run, whether it
allocated the code or was given one with `--code`: `wyrm CODE` (or
`wormhole receive CODE`). After `receive --new` the receiver prints
`wyrm send --code CODE FILE`. Under the `wormhole` name the two forms swap
places ([wormhole-shim.md](wormhole-shim.md)). `wyrm send --qr` also
draws `wormhole-transfer:CODE`, the URI the Python client puts in its QR code,
for a phone app to scan; with `--code` it draws the code given. Like
Python's, it is drawn in half blocks with a one-module border and, on a
terminal, in fixed colours (white on near-black), so it scans whatever the
theme; `--ascii` draws `##` instead, which reads right on a dark terminal.
It is encoded at error correction level H (about 30% recoverable), at
version 6 (41 modules) or more, and the wyrm emblem (`assets/logo.png`) is
laid over its middle: a 13 by 14 module patch, 13 characters by 7 lines,
starting on an even row so it fills whole characters. Only the modules under
the round badge are replaced; around it the code's own pattern runs on. The
patch is 12% of a version 6 code at most, it never covers a finder, timing,
format or version module, and it takes the spot nearest the centre where the
badge covers no alignment pattern, within 8 modules of it. Versions 7 to 13
have no such spot (their patterns near the middle leave gaps too narrow), so
there the badge covers one. A code too long for level H (over 1255 characters)
falls back to the `qrcode` default level with no emblem.

How the emblem looks depends on the terminal (`qr::Look::choose`):

- **Picture.** When `TERM` names kitty, Ghostty or WezTerm (`xterm-kitty`,
  `xterm-ghostty`, `wezterm`; WezTerm's default `xterm-256color` does not
  count) and wyrm is not nested in tmux, screen, zellij, Neovim or Emacs
  (`TMUX`, `STY`, `ZELLIJ`, `NVIM`, `INSIDE_EMACS`), the PNG built into the
  binary (`crates/cli/assets/qr-wyrm.png`, transparent around the badge) is
  placed over the patch with the kitty graphics protocol, once the whole code
  is drawn: the cursor goes back up to the patch, the image goes out in
  base64 chunks of 4096 bytes, sized in cells, leaving the cursor where it
  was (`C=1`) and with replies off (`q=2`), and the cursor comes back down.
  Drawing it last matters to WezTerm, which keeps an image in the cells and
  loses it to text written over them. `TERM_PROGRAM` and `KITTY_WINDOW_ID` are
  not trusted: a terminal started from Ghostty or kitty inherits them.
- **Coloured.** On other terminals each covered module is drawn in the
  emblem's 256-colour pixel.
- **Plain** (not a terminal) and **`--ascii`.** A covered module is light or
  dark by its pixel's brightness.

`scripts/qr-logo.py` (run by uv, Pillow pinned) builds the logo, the PNG and
the pixels (`crates/cli/src/qr/logo.rs`) from `assets/emblem.png`, the
emblem on a white background, turning the white outside its circle
transparent; the same Pillow gives the same files. `qr::grid` builds the
module grid, emblem included, from the `qrcode` crate's; `qr::render` draws
it. There is no
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
- [crates/cli/src/qr/logo.rs](../../crates/cli/src/qr/logo.rs), [scripts/qr-logo.py](../../scripts/qr-logo.py)
- [crates/cli/src/transfer.rs](../../crates/cli/src/transfer.rs) (`progress`, `follow_sent`)
- [crates/cli/src/wyrm.rs](../../crates/cli/src/wyrm.rs)
- [crates/transport-iroh/src/blobs.rs](../../crates/transport-iroh/src/blobs.rs) (`count_sent`)
- [crates/cli/src/receive.rs](../../crates/cli/src/receive.rs) (`prompt_code`, `CodeCompleter`)
- [crates/cli/src/send.rs](../../crates/cli/src/send.rs) (`paths_payload`)
- [crates/cli/src/zipdir.rs](../../crates/cli/src/zipdir.rs) (`build_bundle`)
- [crates/wormhole-core/src/code.rs](../../crates/wormhole-core/src/code.rs)
