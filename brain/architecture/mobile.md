# The Android app

`apps/mobile` is a Flutter project, Android only for now, over
`crates/mobile` (package `wyrmyon-mobile`), the app's Rust side. Rust does the
whole transfer through the transfer library ([transfer.md](transfer.md)); Dart
picks files, scans codes, shows progress and saves what arrives; a little
Kotlin reaches what Flutter does not (the picker, MediaStore, intents, the
foreground service). It sends and receives text, files, several files and
folders.

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
| `completions(typed)` | The words of the PGP word list that complete a typed code, as `wormhole receive` offers on Tab (two-word codes) |
| `Transfer` | A handle per transfer: `cancel()`, and `offer()` once an offer has arrived |
| `send_file(settings, code, name, size, open, transfer)` | Sends one file. `open` is a Dart callback returning a fresh file descriptor each time Rust opens the source (Android hands out `content://` URIs, not paths), so the source stays reopenable for hashing then sending |
| `send_text(settings, code, text, transfer)` | Sends text |
| `send_files(settings, code, name, bundle, files, dirs, open, cache_dir, transfer)` | Sends a tree ([transfer.md](transfer.md)): `files` are each a relative path and a size, `open(index)` returns a descriptor for one of them, `dirs` are empty folders. `bundle` says the files go straight into the receiver's folder rather than into one named `name`. The legacy zip is built in `cache_dir` |
| `receive(settings, code, transfer)` | Pairs and waits for the offer, which the handle then holds: text, acknowledged at once; a file; a folder or several files (`OfferKind::Folder` or `Files`, with their count). Anything else is refused. The offered name keeps only its last printable part (`../../etc/passwd` is `passwd`, an empty one `received`) |
| `accept(transfer, fd, cache_dir)` | Receives a file into a descriptor Dart opened for writing (a MediaStore entry), flushing before the ack, reporting on `receive`'s stream |
| `accept_files(transfer, open, cache_dir)` | Receives a folder or several files, calling `open(path)` for a descriptor per file; a collection arrives over iroh, the legacy zip is received into `cache_dir` and unpacked from there |
| `refuse(transfer)` | Answers `transfer rejected` |

With no `code`, Rust allocates one and reports it as the first update, for
the phone to show; with one, it joins. The app advertises iroh-v1 and
`collection-v1`, as `wyrm` does, so a wyrm sends it folders as a collection,
and it reaches legacy clients over classic transit with the zip.

`send_*` and `receive` return a stream of `Update { stage, bytes, total,
detail }`: `Code` (the code in `detail`), `Notice` (the server's MOTD),
`Paired`, `Hashing`, `Connected` (the route in `detail`), `Moving`,
`AwaitingAck` (classic transit only), then `Offer`, `Done` or `Failed` (the
error in `detail`). Every outcome travels on the stream because frb drops
both the return value and the error of a function that also takes a
`StreamSink`. Progress is sent at most every 100 ms, and always when the
count reaches the total. A plain struct with a simple enum, because frb turns
enums with data into freezed classes, which would add a Dart code generator.

Cancelling goes through the transfer library's `Events::cancelled`
([transfer.md](transfer.md)) once the peers are paired, so a pipe or an iroh
endpoint is closed rather than dropped; before that (waiting for the peer or
for the offer) the pending future is dropped. Either way the peer is sent
`transfer cancelled` once the wormhole is open. An offer that arrives after
the cancel is refused by `receive` itself, and the app also calls `refuse`
when it cancels before answering, so an offer never waits in the handle
unanswered.

The core (`send.rs`, `receive.rs`, `report.rs`, `session.rs`) takes a
`wyrmyon_transfer::Source`, any writer and a callback, so it is tested in
Rust against the in-process mailbox; `api/bridge.rs` turns descriptors
and Dart callbacks into those, owns every descriptor it is handed (a late
`accept` closes it), and is left out of coverage with the generated code.
The app's callbacks never throw into Rust: a failure is a `-1` descriptor. Descriptors are Unix only: elsewhere `accept` and a descriptor source
fail, so the crate still builds in the desktop CI jobs.

## The Android side

`MainActivity.kt` serves the `wyrmyon/platform` method channel, wrapped in
Dart by `lib/platform.dart`:

- **Picking**: `ACTION_OPEN_DOCUMENT` with several files allowed; each comes
  back as its URI, display name and size (from `OpenableColumns`, else the
  descriptor's `statSize`; a file whose size stays unknown cannot be sent).
  `ACTION_OPEN_DOCUMENT_TREE` picks a folder, walked off the UI thread
  through `DocumentsContract` into its files (each with its path inside the
  folder) and its empty folders. `openRead` hands Rust a detached
  descriptor.
- **Intents**: `ACTION_SEND` and `ACTION_SEND_MULTIPLE` (files, or text) and
  `ACTION_VIEW` on `wormhole-transfer:` links. The intent the app started with
  is taken once by Dart; later ones (`singleTop`) are pushed to it. A leader
  link opens Send with the code set, any other link starts receiving.
- **Downloads** (`Downloads.kt`): a received file is inserted into
  `MediaStore.Downloads` under `Download/wyrmyon/` (and the folders of its
  path, for a tree) with `IS_PENDING`, written through a descriptor, then
  published, or deleted when the transfer fails. MediaStore renames a clash
  (`notes (1).txt`) and holds files only, so a tree's empty folders are not
  kept. Home lists the newest entries under that folder rather than a
  history; Android only shows an app the entries it created, so files from
  before a reinstall are not listed. Opening one is an `ACTION_VIEW` with
  its MIME type; the manifest declares `REQUEST_INSTALL_PACKAGES`, without
  which Android's installer silently refuses an APK opened from the app.
- **The service** (`TransferService.kt`): a `dataSync` foreground service runs
  from the start of a transfer to its end, so it survives the screen turning
  off, with an ongoing notification showing progress (updated at most every
  second); a finished or failed transfer posts a notification that opens the
  app. Notifications are asked for at the first transfer (Android 13+).
  It counts the transfers running, so the first to end does not stop it
  under another, and stops through its own `onStartCommand`, so a transfer
  that ends at once never stops it before it went to the foreground. The
  count is dropped with the activity, whose engine the transfers die with.
  Flutter's own deep linking is off: the app routes `wormhole-transfer:`
  links itself.

## The flows

`lib/engine.dart` wraps the bridge behind `Engine` and `Running`, so the
widget tests drive every screen with a fake. `TransferController`
(`lib/transfer.dart`) turns the updates into a phase (waiting, offered,
running, done, failed, cancelled), creates and finishes the download, and
drives the service.

- **Home**: Send files (the picker, then Send) and Receive (the scanner), the
  ten latest received files below, with their folder; a tap opens one, the
  share button passes it on, and All lists every one with Show in folder
  (Android's Downloads app). It reloads when the app resumes or a screen
  closes over it.
- **Settings** (the gear on Home): the mailbox, the transit relay and the
  iroh relays, as the CLI's flags set them. They are checked by
  `check_settings` before they are kept (shared_preferences; the defaults are
  not stored, so they follow the app), and Restore the defaults puts the
  public servers back. The save folder is fixed, so Home can list it.
- **Send**: the files chosen (from the picker or the share menu) or the
  shared text; more files and folders can be added. One file goes as a
  file, one folder as a folder, any other mix as a bundle (which a legacy
  receiver, knowing no bundles, unpacks into a folder named after the count,
  `2 items`); two items with one name, or text with files, cannot be sent.
  Then scan the computer's code, type it, or show a code (with
  `wyrm receive CODE` and its QR code) for a computer to type.
- **Scanner** (mobile_scanner): accepts a code meant for the current way and
  names the other screen otherwise; typing is the fallback, with the word
  list completing each word.
- **Transfer**: the code or "waiting", the server's notice, the offer with
  Accept and Decline (a folder lands in `Download/wyrmyon/NAME`, several files
  straight in `Download/wyrmyon`), progress with the speed and the route,
  and for several files each one ticked off (a sent tree's ticks follow the
  bytes moved, so over the compressed zip they are approximate). Then how
  long the data took and "Verified end to end" (classic transit's SHA-256
  ack, iroh's BLAKE3), with Open, Share or Share all, and Show in folder for
  what arrived, Send more for what left (each code works once, so it picks
  files and asks for a new code), or the text with Copy. Leaving cancels,
  and deletes what a failed or cancelled transfer had written.

## The look

The screens follow the mockups agreed for M11: always dark, a navy radial
gradient behind every screen, Ubuntu and Ubuntu Mono (bundled under the Ubuntu
Font Licence, `assets/fonts/UFL.txt`), a blue-to-cyan gradient for the main
action and outlined cards for the rest, square icon tiles, uppercase section
labels, a pill naming the transport (iroh with a wyrmyon peer, or classic
transit) on the offer and during the transfer, a gradient progress ring, and
a cyan reticle with a sweeping line over the camera. `lib/ui.dart` holds the
palette and those pieces; the pages only compose them. Home shows the dragon
(`assets/logo.png`, a link to the repository's) with a cyan glow, and the
launch screen is the same navy with the launcher icon's dragon, so nothing
flashes white while Flutter starts. The offer says how it came
(`OfferInfo::iroh`); during a transfer the route's text gives the status line
(direct or via relay).

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

The NDK and compile SDK are Flutter's defaults (NDK 28.2, SDK 36); the
minimum is Android 10 (SDK 29), the first with `MediaStore.Downloads`. Gradle 9.1 cannot run on Java newer than 25, so `.mise.toml` pins
Temurin 21 beside Flutter 3.44.6. A release APK carries
`libwyrmyon_mobile.so` and mobile_scanner's bundled ML Kit barcode model (so
scanning works without Google Play services).

The launcher icon is the project's dragon (`assets/logo.png` at the root),
generated by `flutter_launcher_icons`: an adaptive icon with the dragon,
unsmoothed, on `#151B32`. `dart run flutter_launcher_icons` in
`apps/mobile` regenerates it.

Release builds are signed as coincherie's and nostragoalus's are: from
`apps/mobile/android/key.properties` (gitignored) or the
`WYRMYON_ANDROID_KEYSTORE`, `_STORE_PASSWORD`, `_KEY_ALIAS` and
`_KEY_PASSWORD` variables; the keystore lives outside the repository
(`~/.keys/wyrmyon.jks`, alias `wyrmyon`, its properties beside it in
`~/.keys/wyrmyon.properties` to copy into a checkout). Without it a release
build falls back to the debug key, so `flutter run --release` and CI still
build. `mise run apk-release` builds the APKs to publish, one per ABI (about
40 MB each), and refuses without the keystore or if an APK came out
debug-signed. Debug builds are `io.github.arzaroth.wyrmyon.debug`, so they
install beside a release one instead of over it.

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
- [crates/mobile/src/cancel.rs](../../crates/mobile/src/cancel.rs)
- [apps/mobile/lib/engine.dart](../../apps/mobile/lib/engine.dart)
- [apps/mobile/lib/transfer.dart](../../apps/mobile/lib/transfer.dart)
- [apps/mobile/lib/platform.dart](../../apps/mobile/lib/platform.dart)
- [apps/mobile/lib/app.dart](../../apps/mobile/lib/app.dart)
- [apps/mobile/lib/ui.dart](../../apps/mobile/lib/ui.dart)
- [apps/mobile/lib/pages/](../../apps/mobile/lib/pages/)
- [apps/mobile/android/app/src/main/kotlin/io/github/arzaroth/wyrmyon/](../../apps/mobile/android/app/src/main/kotlin/io/github/arzaroth/wyrmyon/)
- [apps/mobile/android/app/src/main/AndroidManifest.xml](../../apps/mobile/android/app/src/main/AndroidManifest.xml)
- [.mise.toml](../../.mise.toml)
- [.github/workflows/ci.yml](../../.github/workflows/ci.yml)
