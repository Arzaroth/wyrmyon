# The Android app

`apps/mobile` is a Flutter project, Android only for now, over
`crates/mobile` (package `wyrmyon-mobile`), the app's Rust side. Rust does the
whole transfer through the transfer library ([transfer.md](transfer.md)); Dart
picks files, scans codes, shows progress and saves what arrives. The flows
(the share menu, the scanner, saving into `Download/wyrmyon`) are the next
step of ROADMAP M11; for now the home screen shows Send and Receive, disabled.

## The bridge

flutter_rust_bridge 2.13.0 generates both sides from `crates/mobile/src/api`
(`flutter_rust_bridge.yaml` in `apps/mobile`): `crates/mobile/src/frb_generated.rs`
and `apps/mobile/lib/src/rust/`. Both are committed; after changing the API,
run `flutter_rust_bridge_codegen generate` in `apps/mobile` with the same
version as the `flutter_rust_bridge` pins in `pubspec.yaml` and
`crates/mobile/Cargo.toml`, which must match exactly.

| Item | What it does |
| --- | --- |
| `default_settings()` | The public mailbox and transit relay, n0's iroh relays |
| `scan(text)` | A scanned `wormhole-transfer:CODE` link, or a typed code, into the code and what the phone does: send for `?role=leader` (`wyrm receive --new --qr`), receive otherwise; anything else is `this is not a wormhole code` |
| `send_file(settings, code, name, size, open, updates)` | Sends one file. `open` is a Dart callback returning a fresh file descriptor each time Rust opens the source (Android hands out `content://` URIs, not paths), so the source stays reopenable for hashing then sending |
| `send_text(settings, code, text, updates)` | Sends text |
| `receive(settings, code, updates)` | Pairs and waits for the offer. Text is acknowledged at once and the transfer ends; a file comes back as an `Incoming` to accept or refuse; folders, several files and anything else are refused (`the app cannot receive folders or several files yet`) |
| `offer_info(incoming)` | The offer: text, or a file's name and size. The name keeps only its last printable part (`../../etc/passwd` is `passwd`, an empty one `received`) |
| `accept(incoming, fd, cache_dir)` / `refuse(incoming)` | Receives into a descriptor Dart opened for writing (a MediaStore entry), flushing before the ack; or answers `transfer rejected` |

With no `code`, Rust allocates one and reports it as the first update, for
the phone to show; with one, it joins. The app advertises iroh-v1 but no
offers (`config_offering(&[])`), so a wyrm never sends it a collection, and
it reaches legacy clients over classic transit.

Every call streams `Update { stage, bytes, total, detail }` to Dart:
`Code` (the code in `detail`), `Notice` (the server's MOTD), `Paired`,
`Hashing`, `Connected` (the route in `detail`), `Moving`, `AwaitingAck`
(classic transit only), `Done`. Progress is sent at most every 100 ms, and
always when the count reaches the total. A plain struct with a simple enum,
because frb turns enums with data into freezed classes, which would add a
Dart code generator.

The core (`send.rs`, `receive.rs`, `report.rs`, `session.rs`) takes a
`wyrmyon_transfer::Source`, any writer and a callback, so it is tested in
Rust against the in-process mailbox; `api/bridge.rs` only turns descriptors
and Dart callbacks into those, and is left out of coverage with the generated
code. Descriptors are Unix only: elsewhere `accept` and a descriptor source
fail, so the crate still builds in the desktop CI jobs.

## The build

The Rust library is a native asset: `apps/mobile/hook/build.dart` runs
`flutter_rust_bridge_hooks` (on `native_toolchain_rust`), so every
`flutter run`, `flutter test` and `flutter build apk` builds `crates/mobile`
for its targets with `rustup run <channel> cargo build`, into the hook's own
target directory. Rather than cargokit, which would vendor a Gradle plugin
and its scripts into the repository. The hook requires, in `crates/mobile`:

- `crate-type` with `cdylib` and `staticlib` (`rlib` too, for the Rust tests);
- a `rust-toolchain.toml` with an exact channel, the same as the workspace's,
  listing the targets: the three Android ABIs (`aarch64-linux-android`,
  `armv7-linux-androideabi`, `x86_64-linux-android`) and
  `x86_64-unknown-linux-gnu`, so `flutter test` on a Linux host loads the
  library and calls Rust (`test/bridge_test.dart` opens
  `build/native_assets/linux/libwyrmyon_mobile.so`). Desktop builds from the
  workspace root keep using the root `rust-toolchain.toml`, so the Android
  targets are only downloaded for the app.

The NDK, compile and minimum SDK are Flutter's defaults (NDK 28.2, SDK 36,
min 24). Gradle 9.1 cannot run on Java newer than 25, so `.mise.toml` pins
Temurin 21 beside Flutter 3.44.6. The release APK carries
`libwyrmyon_mobile.so` for the three ABIs, about 20 MB each, and is still
signed with the debug key. The manifest asks for `INTERNET` only.

CI's `android` job installs Flutter and Java through mise, then runs
`flutter analyze`, `flutter test` and `flutter build apk --release`, and
checks the APK holds the arm64 library. The generated bridge is not
regenerated in CI.

## Sources

- [crates/mobile/src/api/types.rs](../../crates/mobile/src/api/types.rs)
- [crates/mobile/src/api/bridge.rs](../../crates/mobile/src/api/bridge.rs)
- [crates/mobile/src/send.rs](../../crates/mobile/src/send.rs)
- [crates/mobile/src/receive.rs](../../crates/mobile/src/receive.rs)
- [crates/mobile/src/report.rs](../../crates/mobile/src/report.rs)
- [crates/mobile/src/session.rs](../../crates/mobile/src/session.rs)
- [crates/mobile/rust-toolchain.toml](../../crates/mobile/rust-toolchain.toml)
- [apps/mobile/hook/build.dart](../../apps/mobile/hook/build.dart)
- [apps/mobile/flutter_rust_bridge.yaml](../../apps/mobile/flutter_rust_bridge.yaml)
- [apps/mobile/lib/main.dart](../../apps/mobile/lib/main.dart)
- [.mise.toml](../../.mise.toml)
- [.github/workflows/ci.yml](../../.github/workflows/ci.yml)
