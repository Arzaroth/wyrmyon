import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wyrmyon/app.dart';
import 'package:wyrmyon/engine.dart';
import 'package:wyrmyon/platform.dart';

import 'fakes.dart';

const photo = PickedFile(
  uri: 'content://photo',
  name: 'photo.jpg',
  size: 4100000,
);

Future<void> settle(WidgetTester tester) async {
  for (var i = 0; i < 10; i++) {
    await tester.pump(const Duration(milliseconds: 100));
  }
}

Future<(FakeEngine, FakePlatform)> launch(
  WidgetTester tester, {
  Shared? shared,
  List<Download> downloads = const [],
}) async {
  final engine = FakeEngine();
  final platform = FakePlatform()
    ..initial = shared
    ..downloads = downloads;
  await tester.pumpWidget(WyrmyonApp(engine: engine, platform: platform));
  await settle(tester);
  return (engine, platform);
}

void main() {
  testWidgets('home offers both ways and lists the latest received files', (
    tester,
  ) async {
    final (_, platform) = await launch(
      tester,
      downloads: [
        Download(
          uri: 'content://downloads/9',
          name: 'invoice.pdf',
          size: 312000,
          modified: DateTime.now(),
        ),
      ],
    );
    expect(find.text('Send files'), findsOneWidget);
    expect(find.text('Receive'), findsOneWidget);
    expect(find.text('invoice.pdf'), findsOneWidget);
    await tester.tap(find.text('invoice.pdf'));
    await tester.tap(find.byTooltip('Share'));
    expect(platform.calls, [
      'open content://downloads/9',
      'share content://downloads/9',
    ]);
  });

  testWidgets('home says where files land before any arrives', (tester) async {
    await launch(tester);
    expect(find.text('Nothing received yet'), findsOneWidget);
  });

  testWidgets('a shared file is sent to a code the phone shows', (
    tester,
  ) async {
    final (engine, platform) = await launch(
      tester,
      shared: const SharedFiles([photo]),
    );
    expect(find.text('photo.jpg'), findsOneWidget);
    await tester.tap(find.text('Show a code'));
    await settle(tester);
    expect(engine.runs.single, (kind: 'file', code: null, what: 'photo.jpg'));

    engine.running.tell(Stage.code, detail: '7-guitarist-revenge');
    await tester.pump();
    expect(find.text('7-guitarist-revenge'), findsOneWidget);
    expect(find.text('wyrm receive 7-guitarist-revenge'), findsOneWidget);

    engine.running
      ..tell(Stage.paired)
      ..tell(Stage.connected, total: 4100000, detail: 'iroh, direct')
      ..tell(Stage.moving, bytes: 2050000, total: 4100000);
    await tester.pump();
    expect(find.text('50%'), findsOneWidget);
    expect(find.text('iroh, direct'), findsOneWidget);

    engine.running.tell(Stage.done);
    await settle(tester);
    expect(find.text('Sent'), findsOneWidget);
    expect(platform.calls, [
      'started Sending photo.jpg',
      'ended Sending photo.jpg: Sent photo.jpg',
    ]);
  });

  testWidgets('a shared text is sent', (tester) async {
    final (engine, _) = await launch(tester, shared: const SharedText('hi'));
    await tester.tap(find.text('Show a code'));
    await settle(tester);
    expect(engine.runs.single, (kind: 'text', code: null, what: 'hi'));
  });

  testWidgets('a leader link sends the picked file to its code', (
    tester,
  ) async {
    final (engine, platform) = await launch(
      tester,
      shared: const SharedLink(
        'wormhole-transfer:7-guitarist-revenge?role=leader',
      ),
    );
    platform.picked = const [photo];
    await tester.tap(find.text('Files'));
    await settle(tester);
    await tester.tap(find.text('Send to 7-guitarist-revenge'));
    await settle(tester);
    expect(engine.runs.single, (
      kind: 'file',
      code: '7-guitarist-revenge',
      what: 'photo.jpg',
    ));
  });

  testWidgets('an accepted file is saved into the downloads', (tester) async {
    final (engine, platform) = await launch(
      tester,
      shared: const SharedLink('wormhole-transfer:7-guitarist-revenge'),
    );
    expect(engine.runs.single, (
      kind: 'receive',
      code: '7-guitarist-revenge',
      what: '',
    ));
    engine.running
      ..offered = OfferInfo(
        kind: OfferKind.file,
        name: 'notes.txt',
        size: BigInt.from(2000),
        count: BigInt.from(1),
        text: '',
      )
      ..tell(Stage.offer);
    await tester.pump();
    await tester.tap(find.text('Accept'));
    await settle(tester);
    expect(engine.running.accepted, (fd: 7, cacheDir: '/cache/received'));

    engine.running
      ..tell(Stage.moving, bytes: 2000, total: 2000)
      ..tell(Stage.done);
    await settle(tester);
    expect(find.text('Received'), findsOneWidget);
    await tester.tap(find.text('Open'));
    expect(platform.calls, [
      'started Receiving',
      'create notes.txt',
      'finish content://downloads/1 kept=true',
      'ended Receiving notes.txt: Saved notes.txt to Download/wyrmyon',
      'open content://downloads/1',
    ]);

    platform.downloads = [
      Download(
        uri: 'content://downloads/1',
        name: 'notes.txt',
        size: 2000,
        modified: DateTime.now(),
      ),
    ];
    await tester.tap(find.text('Done'));
    await settle(tester);
    expect(find.text('notes.txt'), findsOneWidget);
  });

  testWidgets('a failed download is removed and the error shown', (
    tester,
  ) async {
    final (engine, platform) = await launch(
      tester,
      shared: const SharedLink('wormhole-transfer:7-guitarist-revenge'),
    );
    engine.running
      ..offered = OfferInfo(
        kind: OfferKind.file,
        name: 'notes.txt',
        size: BigInt.from(2000),
        count: BigInt.from(1),
        text: '',
      )
      ..tell(Stage.offer);
    await tester.pump();
    await tester.tap(find.text('Accept'));
    await settle(tester);
    engine.running.tell(Stage.failed, detail: 'the sender went away');
    await settle(tester);
    expect(find.text('the sender went away'), findsOneWidget);
    expect(platform.calls, contains('finish content://downloads/1 kept=false'));
  });

  testWidgets('declining an offer refuses it', (tester) async {
    final (engine, platform) = await launch(
      tester,
      shared: const SharedLink('wormhole-transfer:7-guitarist-revenge'),
    );
    engine.running
      ..offered = OfferInfo(
        kind: OfferKind.file,
        name: 'notes.txt',
        size: BigInt.from(2000),
        count: BigInt.from(1),
        text: '',
      )
      ..tell(Stage.offer);
    await tester.pump();
    await tester.tap(find.text('Decline'));
    await settle(tester);
    expect(engine.running.refused, isTrue);
    expect(platform.calls.where((c) => c.startsWith('create')), isEmpty);
    expect(find.text('Send files'), findsOneWidget);
  });

  testWidgets('a received text is shown', (tester) async {
    final (engine, _) = await launch(
      tester,
      shared: const SharedLink('wormhole-transfer:7-guitarist-revenge'),
    );
    engine.running
      ..offered = OfferInfo(
        kind: OfferKind.text,
        name: '',
        size: BigInt.from(5),
        count: BigInt.from(1),
        text: 'hello',
      )
      ..tell(Stage.done)
      ..tell(Stage.offer);
    await settle(tester);
    expect(find.text('Received'), findsOneWidget);
    expect(find.text('hello'), findsOneWidget);
  });

  testWidgets('cancelling stops the transfer and leaves', (tester) async {
    final (engine, platform) = await launch(
      tester,
      shared: const SharedLink('wormhole-transfer:7-guitarist-revenge'),
    );
    await tester.tap(find.text('Cancel'));
    await settle(tester);
    expect(engine.running.cancelled, isTrue);
    expect(platform.calls.last, 'ended Receiving: null');
    expect(find.text('Send files'), findsOneWidget);
  });

  testWidgets('a typed code completes from the word list', (tester) async {
    final (_, platform) = await launch(
      tester,
      shared: const SharedFiles([photo]),
    );
    await tester.tap(find.text('Type a code'));
    await settle(tester);
    await tester.enterText(find.byType(TextField), '7-gu');
    await tester.pump();
    await tester.tap(find.text('guitarist'));
    await tester.pump();
    await tester.enterText(find.byType(TextField), '7-guitarist-revenge');
    await tester.pump();
    await tester.tap(find.text('Connect'));
    await settle(tester);
    expect(find.text('Sending'), findsOneWidget);
    expect(platform.calls.first, 'started Sending photo.jpg');
  });
}
