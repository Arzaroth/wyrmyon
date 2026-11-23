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
      turns them off). A transfer is one offer, so one control stream is all
      it needs, several files included (see M8).
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
      paths travel as one directory named `files`. Revised: between two wyrms
      they land loose, merged into the receiver's directories, and `send -R`
      keeps paths as typed; legacy receivers still get `files/`.
- [x] Code prompt with word completion (rustyline, Tab); prefer it over codes
      on the command line, which leak into `ps` and shell history.
- [x] Receiver-first mode (`wyrm receive --new`), indicatif progress bars
      (both landed with M3).

## M7 - Packaging

- [x] maturin with `bindings = "bin"`: the wheels build (CI checks it);
      publishing them to PyPI is deferred, see Later.
- [x] GitHub releases: archives, install scripts, .deb, .rpm, Arch packages
      and PKGBUILDs, a Windows MSI, man pages and completions; the release
      workflow and `scripts/release.sh`. Decided: our own workflow rather than
      cargo-dist (it has no deb, rpm or Arch); static musl on Linux; x86_64
      and arm64 on Linux, macOS and Windows; macOS gets tarballs and the
      install script, no unsigned .pkg or .dmg; the MSI is unsigned;
      every package but the aarch64 Arch one is installed on a matching
      system before a release.

## M8 - Collections

- [x] A directory or several paths between two wyrms travel as an iroh-blobs
      collection: one blob per file, no temporary zip, each file verified.
      Decided: an `offers: ["collection-v1"]` advert beside `transports`, so
      older wyrms and legacy clients keep the zip; the zip is built only once
      the peer is known; our own JSON manifest as the first child, checked
      against the offer before any file data; the control stream unchanged.

## M9 - Several receivers

- [x] `wyrm send --times N` serves N receivers with one code, in turn.
      Decided: a random 7-digit nameplate, past the server's allocator; a
      fresh mailbox connection per receiver; the payload staged once; a
      wrong code ends the run so the code allows one guess.

## M10 - Port forwarding

- [x] `wyrm forward PORT...` shares local ports, `wyrm forward CODE [LOCAL:]PORT...`
      listens on them, over iroh only. Decided: the taker names ports, never
      hosts; one QUIC stream per TCP connection with a port header and a
      status byte; the mailbox closed once connected.

## M11 - Android app

Files between a phone and a computer, the computer showing the code and the
phone scanning it, both ways. Decided: Flutter over the Rust crates through
flutter_rust_bridge, Android first; the app lives in this repository; on the
phone the files are chosen before the code is scanned.

- [x] `wyrm receive --new --qr` draws the code with `role=leader`, which asks
      the scanning app to send.
- [x] The transfer engine as a library, `crates/transfer`, shared by the CLI
      and the app: it sends a `Source` (a path, or any stream, as Android
      hands out `content://` URIs) and receives into a `Sink` (a file, or any
      writer). Decided: streams rather than copying to a path first; iroh
      still copies a stream into its blob store, as it needs the hash before
      the offer.
- [x] `crates/mobile`: the flutter_rust_bridge API (send sources, receive a
      code, an offer to accept, progress events) over the existing crates.
- [x] `apps/mobile`: the Flutter project, Android only; Flutter, Java and the
      Android Rust targets pinned; a CI job for `flutter test` and the APK.
- [x] The flows: the share menu and the picker into Send, then the scanner
      (or typing the code, or showing one); Receive by scanning or typing,
      with the offer to accept; files saved to `Download/wyrmyon` through
      MediaStore and the newest of them listed on Home, read from the folder
      rather than kept as a history; a foreground service and a notification
      when a transfer ends; `wormhole-transfer:` links open the app in either
      role.
- [ ] Several files and folders from streams: the collection import and the
      legacy zip built from sources, and a received collection or zip written
      into a sink; then folders through Android's folder picker.

## Later

- [ ] `--times` receivers served at once rather than in turn: one
      mailbox connection per pairing in flight, and progress bars that can
      share the terminal.

- [ ] Publish the wheels to PyPI (`uvx wyrmyon`, `pipx install wyrmyon`) and
      a Homebrew formula in `Arzaroth/homebrew-tap`: needs the PyPI project
      with trusted publishers and the tap repository with a push token; how
      to wire them back is in `brain/architecture/distribution.md`.
- [ ] Push the rendered `wyrmyon` and `wyrmyon-bin` PKGBUILDs to the AUR.
- [ ] Fallback mailbox of our own, used only when the public one is
      unreachable; its codes carry a prefix (`m7-guitarist-revenge`) that
      legacy clients reject as malformed, which is correct.
- [ ] Dilation on the legacy side, for Python clients that support it.
- [ ] Paired devices: a phone and a computer that met once by QR code
      remember each other's node IDs and a shared key, and send over iroh
      without a code to a `wyrm` left listening.
- [ ] The app on iOS, from the same Flutter project.
- [ ] Self-hosted iroh relays; a client default change, since the relay URL
      travels inside the encrypted NodeAddr.

## Not planned

- Our own mailbox as the default rendezvous: the sender cannot know which
  client the receiver runs, so codes must live where everyone looks.
- A protocol choice exposed to the user beyond the debugging flags.
