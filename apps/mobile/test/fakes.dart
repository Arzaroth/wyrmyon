import 'dart:async';

import 'package:wyrmyon/engine.dart';
import 'package:wyrmyon/platform.dart';

class FakeRunning implements Running {
  final _updates = StreamController<Update>();
  OfferInfo? offered;
  bool cancelled = false;
  bool refused = false;
  ({int fd, String cacheDir})? accepted;

  @override
  Stream<Update> get updates => _updates.stream;

  void tell(Stage stage, {int bytes = 0, int total = 0, String detail = ''}) {
    _updates.add(
      Update(
        stage: stage,
        bytes: BigInt.from(bytes),
        total: BigInt.from(total),
        detail: detail,
      ),
    );
  }

  @override
  void cancel() => cancelled = true;

  @override
  OfferInfo? offer() => offered;

  @override
  Future<void> accept(int fd, String cacheDir) async {
    accepted = (fd: fd, cacheDir: cacheDir);
  }

  @override
  Future<void> refuse() async => refused = true;
}

class FakeEngine implements Engine {
  final runs = <({String kind, String? code, String what})>[];
  final running = FakeRunning();

  @override
  Scanned? scan(String text) {
    final link = text.trim();
    final bare = link.startsWith('wormhole-transfer:')
        ? link.substring('wormhole-transfer:'.length)
        : link;
    final (code, leader) = bare.endsWith('?role=leader')
        ? (bare.substring(0, bare.length - '?role=leader'.length), true)
        : (bare, false);
    if (!RegExp(r'^\d+-\w+-\w+$').hasMatch(code)) return null;
    return Scanned(
      code: code,
      direction: leader ? Direction.send : Direction.receive,
    );
  }

  @override
  List<String> completions(String typed) =>
      typed == '7-gu' ? ['7-guitarist-', '7-gunpowder-'] : const [];

  @override
  Running sendFile({
    String? code,
    required String name,
    required int size,
    required Future<int> Function() open,
  }) {
    runs.add((kind: 'file', code: code, what: name));
    return running;
  }

  @override
  Running sendText({String? code, required String text}) {
    runs.add((kind: 'text', code: code, what: text));
    return running;
  }

  @override
  Running receive({String? code}) {
    runs.add((kind: 'receive', code: code, what: ''));
    return running;
  }
}

class FakePlatform implements Platform {
  final calls = <String>[];
  final sharing = StreamController<Shared>.broadcast();
  Shared? initial;
  List<PickedFile> picked = const [];
  List<Download> downloads = const [];

  @override
  Stream<Shared> get shared => sharing.stream;

  @override
  Future<Shared?> takeShared() async => initial;

  @override
  Future<List<PickedFile>> pickFiles() async => picked;

  @override
  Future<int> openRead(String uri) async => 3;

  @override
  Future<String> cacheDir() async => '/cache';

  @override
  Future<({String uri, int fd})> createDownload(String name) async {
    calls.add('create $name');
    return (uri: 'content://downloads/1', fd: 7);
  }

  @override
  Future<void> finishDownload(String uri, {required bool kept}) async {
    calls.add('finish $uri kept=$kept');
  }

  @override
  Future<List<Download>> recentDownloads(int limit) async => downloads;

  @override
  Future<bool> openDownload(String uri) async {
    calls.add('open $uri');
    return true;
  }

  @override
  Future<bool> shareDownload(String uri) async {
    calls.add('share $uri');
    return true;
  }

  @override
  Future<void> transferStarted(String title) async =>
      calls.add('started $title');

  @override
  Future<void> transferProgress(
    String title,
    String text,
    int bytes,
    int total,
  ) async {}

  @override
  Future<void> transferEnded(String title, String? text) async =>
      calls.add('ended $title: $text');
}
