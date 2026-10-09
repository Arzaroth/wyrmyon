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

## Several paths

`wyrm send a.txt photos/` (or bare `wyrm a.txt photos/`) zips the paths
side by side, each under its own name, and offers them as one directory named
`files`; any receiver unpacks it like a directory
([directories.md](directories.md)). Two paths with the same name are refused
before anything is sent.

## Sources

- [crates/cli/src/lib.rs](../../crates/cli/src/lib.rs) (`Cli`, `bare`)
- [crates/cli/src/receive.rs](../../crates/cli/src/receive.rs) (`prompt_code`, `CodeCompleter`)
- [crates/cli/src/send.rs](../../crates/cli/src/send.rs) (`paths_payload`)
- [crates/cli/src/zipdir.rs](../../crates/cli/src/zipdir.rs) (`build_bundle`)
- [crates/wormhole-core/src/code.rs](../../crates/wormhole-core/src/code.rs)
