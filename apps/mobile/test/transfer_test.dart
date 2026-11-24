import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:wyrmyon/engine.dart';
import 'package:wyrmyon/transfer.dart';

import 'fakes.dart';

OfferInfo offering(OfferKind kind, String name) => OfferInfo(
  kind: kind,
  name: name,
  size: BigInt.from(10),
  count: BigInt.one,
  text: '',
  iroh: true,
);

Future<(TransferController, FakeEngine, FakePlatform)> offered(
  OfferKind kind,
  String name,
) async {
  final engine = FakeEngine();
  final platform = FakePlatform();
  final transfer = TransferController(
    engine: engine,
    platform: platform,
    job: const Receive(),
  )..start();
  engine.running
    ..offered = offering(kind, name)
    ..tell(Stage.offer);
  await pumpEventQueue();
  expect(transfer.phase, Phase.offered);
  return (transfer, engine, platform);
}

void main() {
  test('cancelling before any answer refuses a late offer', () async {
    final engine = FakeEngine();
    final transfer = TransferController(
      engine: engine,
      platform: FakePlatform(),
      job: const Receive(),
    )..start();
    transfer.cancel();
    await pumpEventQueue();
    expect(engine.running.cancelled, isTrue);
    expect(engine.running.refused, isTrue);
    expect(transfer.phase, Phase.cancelled);
  });

  test('a cancel while the download is made leaves nothing pending', () async {
    final (transfer, engine, platform) = await offered(
      OfferKind.file,
      'notes.txt',
    );
    platform.creating = Completer();
    final accepting = transfer.accept();
    await pumpEventQueue();
    transfer.cancel();
    platform.creating!.complete();
    await accepting;
    expect(platform.calls, contains('finish content://downloads/1 kept=false'));
    expect(engine.running.accepted?.fd, 7);
    expect(engine.running.refused, isFalse);
    expect(platform.calls.where((c) => c.startsWith('ended')), hasLength(1));
    expect(transfer.phase, Phase.cancelled);
  });

  test('a download that cannot be made refuses the offer once', () async {
    final (transfer, engine, platform) = await offered(
      OfferKind.file,
      'notes.txt',
    );
    platform.createFails = Exception('storage full');
    await transfer.accept();
    expect(engine.running.refused, isTrue);
    expect(transfer.phase, Phase.failed);
    expect(transfer.error, contains('storage full'));
    expect(platform.calls.where((c) => c.startsWith('ended')), hasLength(1));
  });

  test(
    'a member that cannot be made is a bad descriptor, not a throw',
    () async {
      final (transfer, engine, platform) = await offered(
        OfferKind.folder,
        'album',
      );
      await transfer.accept();
      platform.createFails = Exception('storage full');
      expect(await engine.running.openMember!('a.jpg'), -1);
    },
  );

  test('a member made after a cancel is deleted and handed back', () async {
    final (transfer, engine, platform) = await offered(
      OfferKind.folder,
      'album',
    );
    await transfer.accept();
    platform.creating = Completer();
    final opening = engine.running.openMember!('a.jpg');
    await pumpEventQueue();
    transfer.cancel();
    platform.creating!.complete();
    expect(await opening, 7);
    expect(platform.calls, contains('finish content://downloads/1 kept=false'));
    expect(transfer.savedUris, isEmpty);
  });
}
