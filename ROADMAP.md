# Roadmap

The agreed backlog, in delivery order, with the decisions taken on each item so
they are not lost. Vocabulary follows [brain/glossary.md](brain/glossary.md);
the reasons behind the shape are in [brain/decisions.md](brain/decisions.md).
Small fixes and open questions go in [TODO.md](TODO.md).

The legacy path comes first, because the Python `wormhole` CLI tests it for
free; the iroh path is then a pure upgrade on top of it.

## M0 - Foundation

- [x] Cargo workspace: `wormhole-core`, `transport-classic`, `transport-iroh`,
      `cli`; the `wyrmyon` and `wyrm` binaries; CI on x86_64 and aarch64.
- [x] Name chosen: wyrmyon, free on crates.io, PyPI, npm and Homebrew. `wyrm`
      ships only as the second binary, never as a package name (taken on
      crates.io and PyPI).

## M1 - Wormhole core

- [x] Mailbox client over WebSocket (tokio-tungstenite): bind, allocate or
      claim a nameplate, open the mailbox, add and receive messages, close.
      Decided: our own implementation, not magic-wormhole.rs (EUPL-1.2).
- [x] SPAKE2 on the code, HKDF phase keys, secretbox, the version exchange.
- [x] Text send and receive against the Python `wormhole` CLI in both
      directions. This is the first interop test. Decided: CI runs against an
      in-process fake mailbox (`crates/testkit`); the interop suite starts the
      real Python mailbox through `uvx` and runs on demand.

## M2 - Classic transit on the LAN

- [x] Transit handshake and direct TCP from the peer's hints, records
      encrypted with the transit key.
- [x] File offer and answer, one file end to end against the Python CLI.
      Decided: typed app messages (`crates/cli/src/protocol.rs`); unattended
      receiving needs `--accept-file`; partial files never overwrite.

## M3 - Full legacy transfer

- [x] Transit relay fallback (`transit.magic-wormhole.io`) and connection
      racing across hints. Decided: relays 2 s after direct hints, both
      peers' relays tried; `--transit-helper`, `--no-listen`.
- [x] Directories (zipped, as the Python client does) and progress.
      Decided: extraction capped at the offered size and count; symlinks
      skipped.
- [x] Receiver-first codes work with a legacy sender
      (`wormhole send --code ...`): `wyrm receive --new`, `wyrm send --code`.

## M4 - The iroh-v1 transport

- [x] Advertise `{"wyrmyon": {"transports": ["iroh-v1"]}}` in `app_versions`;
      both sides pick the transport from the two adverts at the same time.
      Decided: detection happens after PAKE, never before.
- [x] Exchange addresses through the encrypted mailbox and pin the peer's
      Ed25519 node ID. Decided: our own `IrohInfo` wire form, not iroh's
      `EndpointAddr` serde; no pkarr publishing.
- [x] Channel binding: HKDF a confirm key (`wyrmyon/iroh-v1/confirm`) from the
      wormhole key and exchange MACs on the first stream. ALPN `wyrmyon/1`.
- [x] One stream per transfer; n0's public relays (`--iroh-relays disabled`
      turns them off). A transfer is one offer today, so one stream is all it
      needs; a control stream comes with multi-file transfers.
- [x] `--force-classic` / `--force-iroh` for testing. Decided: no fallback to
      classic transit when iroh fails.

## M5 - Verified, resumable transfers

- [x] iroh-blobs: BLAKE3 verified streaming, resume from the last verified
      chunk after a drop. Decided: iroh-blobs 0.103 inside the bound
      connection (no blobs ALPN), the hash on the control stream, one cache
      store per hash under `~/.cache/wyrmyon/partial`.

## M6 - CLI polish

- [x] The argument decides the action: paths send, a code receives, piped
      stdin sends text, nothing prompts. An argument that is both a code and a
      path on disk is refused until a subcommand says which. Decided: several
      paths travel as one directory named `files`.
- [x] Code prompt with word completion (rustyline, Tab); prefer it over codes
      on the command line, which leak into `ps` and shell history.
- [x] Receiver-first mode (`wyrm receive --new`), indicatif progress bars
      (both landed with M3).

## M7 - Packaging

- [x] maturin with `bindings = "bin"`: wheels on PyPI, so `uvx wyrmyon` and
      `pipx install wyrmyon` work without a Python runtime.
- [x] cargo-dist: GitHub release archives, shell and PowerShell installers, a
      Homebrew tap. The release workflow and `scripts/release.sh` land here.
      Decided: Linux (glibc), macOS and Windows on x86_64 and aarch64 (Windows
      x86_64 only), all tested in CI; wheels published by trusted publishing.

## Later

- [ ] Fallback mailbox of our own, used only when the public one is
      unreachable; its codes carry a prefix (`m7-guitarist-revenge`) that
      legacy clients reject as malformed, which is correct.
- [ ] Dilation on the legacy side, for Python clients that support it.
- [ ] Self-hosted iroh relays; a client default change, since the relay URL
      travels inside the encrypted NodeAddr.

## Not planned

- Our own mailbox as the default rendezvous: the sender cannot know which
  client the receiver runs, so codes must live where everyone looks.
- A protocol choice exposed to the user beyond the debugging flags.
