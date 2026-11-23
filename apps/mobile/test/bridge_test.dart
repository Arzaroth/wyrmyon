import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wyrmyon/engine.dart' show RustEngine;
import 'package:wyrmyon/src/rust/api/bridge.dart';
import 'package:wyrmyon/src/rust/api/types.dart';
import 'package:wyrmyon/src/rust/frb_generated.dart';

void main() {
  setUpAll(
    () async => RustLib.init(
      externalLibrary: ExternalLibrary.open(
        'build/native_assets/linux/libwyrmyon_mobile.so',
      ),
    ),
  );

  test('a leader link asks the phone to send', () {
    final scanned = scan(
      text: 'wormhole-transfer:7-guitarist-revenge?role=leader',
    );
    expect(scanned.code, '7-guitarist-revenge');
    expect(scanned.direction, Direction.send);
  });

  test('anything else is refused', () {
    expect(() => scan(text: 'https://example.com'), throwsA(anything));
  });

  test('the defaults are the public servers', () {
    expect(
      defaultSettings().mailbox,
      startsWith('ws://relay.magic-wormhole.io'),
    );
  });

  test('a typed code completes from the word list', () {
    expect(completions(typed: '7-guitar'), contains('7-guitarist-'));
  });

  test('the engine reads codes and refuses the rest', () {
    final engine = RustEngine();
    expect(engine.scan('7-guitarist-revenge')?.direction, Direction.receive);
    expect(engine.scan('https://example.com'), isNull);
  });

  test('a transfer holds no offer until one arrives', () {
    final transfer = Transfer()..cancel();
    expect(transfer.offer(), isNull);
  });
}
