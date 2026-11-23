# wyrmyon for Android

The Flutter app: files between a phone and a computer, the computer showing
the code and the phone scanning it. Its Rust side is
[`crates/mobile`](../../crates/mobile), bridged by flutter_rust_bridge and
built by the Dart build hook in `hook/build.dart`, so `flutter run`,
`flutter test` and `flutter build apk` compile it for each target.

Send files, several files or a folder, from the share menu or the pickers, then scan the computer's code
(`wyrm receive --new --qr`), type it, or show one; receive by scanning
`wyrm send --qr` or typing its code. Files land in `Download/wyrmyon`. Needs
Android 10 or newer. How it fits together: `brain/architecture/mobile.md`.

The toolchain is pinned in the repository's `.mise.toml` (Flutter, Java) and
in `crates/mobile/rust-toolchain.toml` (the Android Rust targets).

```sh
flutter test
flutter build apk
```

After changing the Rust API in `crates/mobile/src/api`, regenerate the bridge
with `flutter_rust_bridge_codegen generate` (version 2.13.0, as pinned in
`pubspec.yaml` and `crates/mobile/Cargo.toml`) and commit the result.
