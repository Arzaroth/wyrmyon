import 'dart:async';

import 'package:wyrmyon/engine.dart';
import 'package:wyrmyon/platform.dart';
import 'package:wyrmyon/settings.dart';

const publicSettings = Settings(
  mailbox: 'ws://relay.magic-wormhole.io:4000/v1',
  transitHelper: 'tcp:transit.magic-wormhole.io:4001',
  irohRelays: 'default',
);

class FakeSettingsStore implements SettingsStore {
  Settings? kept;
  var saves = 0;

  @override
  Future<Settings?> load() async => kept;

  @override
  Future<void> save(Settings? settings) async {
    kept = settings;
    saves++;
  }
}

class FakeRunning implements Running {
  final _updates = StreamController<Update>();
  OfferInfo? offered;
  bool cancelled = false;
  bool refused = false;
  ({int fd, String cacheDir})? accepted;
  Future<int> Function(String path)? openMember;

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
  Future<void> acceptFiles(
    Future<int> Function(String path) open,
    String cacheDir,
  ) async {
    openMember = open;
    accepted = (fd: -1, cacheDir: cacheDir);
  }

  @override
  Future<void> refuse() async => refused = true;
}

class FakeEngine implements Engine {
  final runs = <({String kind, String? code, String what})>[];
  List<TreeFile> sentFiles = const [];
  List<String> sentDirs = const [];
  final running = FakeRunning();

  @override
  Settings get defaults => publicSettings;

  @override
  Settings settings = publicSettings;

  @override
  String? check(Settings settings) => settings.mailbox.startsWith('ws')
      ? null
      : 'the mailbox: a ws:// or wss:// address';

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
  Running sendFiles({
    String? code,
    required String name,
    required bool bundle,
    required List<TreeFile> files,
    required List<String> dirs,
    required Future<int> Function(int index) open,
    required String cacheDir,
  }) {
    runs.add((kind: bundle ? 'bundle' : 'folder', code: code, what: name));
    sentFiles = files;
    sentDirs = dirs;
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
  PickedFolder? folder;
  var created = 0;
  Completer<void>? creating;
  Object? createFails;
  List<Download> downloads = const [];
  var opens = true;

  @override
  Stream<Shared> get shared => sharing.stream;

  @override
  Future<Shared?> takeShared() async => initial;

  @override
  Future<List<PickedFile>> pickFiles() async => picked;

  @override
  Future<PickedFolder?> pickFolder() async => folder;

  @override
  Future<int> openRead(String uri) async => 3;

  @override
  Future<String> cacheDir() async => '/cache';

  @override
  Future<({String uri, int fd})> createDownload(String path) async {
    calls.add('create $path');
    await creating?.future;
    if (createFails case final error?) throw error;
    created++;
    return (uri: 'content://downloads/$created', fd: 6 + created);
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
    return opens;
  }

  @override
  Future<bool> shareDownload(String uri) async {
    calls.add('share $uri');
    return true;
  }

  @override
  Future<bool> shareDownloads(List<String> uris) async {
    calls.add('share ${uris.join(' ')}');
    return true;
  }

  @override
  Future<bool> showDownloads() async {
    calls.add('show downloads');
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
