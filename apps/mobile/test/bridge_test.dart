import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter_test/flutter_test.dart';
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
}
