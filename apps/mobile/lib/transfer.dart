import 'dart:async';

import 'package:flutter/foundation.dart';

import 'engine.dart';
import 'format.dart';
import 'platform.dart';

sealed class Job {
  const Job();
}

class SendFile extends Job {
  const SendFile(this.file);
  final PickedFile file;
}

class SendText extends Job {
  const SendText(this.text);
  final String text;
}

typedef Entry = ({String path, PickedFile file});

class SendTree extends Job {
  const SendTree({
    required this.name,
    required this.bundle,
    required this.entries,
    this.dirs = const [],
  });

  final String name;
  final bool bundle;
  final List<Entry> entries;
  final List<String> dirs;

  int get size => entries.fold(0, (sum, entry) => sum + entry.file.size);
}

class Receive extends Job {
  const Receive();
}

DateTime Function() clock = DateTime.now;

enum Phase { waiting, offered, running, done, failed, cancelled }

class TransferController extends ChangeNotifier {
  TransferController({
    required this.engine,
    required this.platform,
    required this.job,
    this.code,
  });

  final Engine engine;
  final Platform platform;
  final Job job;
  final String? code;

  Phase phase = Phase.waiting;
  Stage? stage;
  String? shownCode;
  String? notice;
  String? route;
  OfferInfo? offer;
  String? error;
  int bytes = 0;
  int total = 0;

  Running? _running;
  StreamSubscription<Update>? _updates;
  ({String uri, int fd})? _download;
  final _downloads = <String>[];
  DateTime? _told;
  bool _disposed = false;

  bool get sending => job is! Receive;
  bool get ended =>
      phase == Phase.done || phase == Phase.failed || phase == Phase.cancelled;
  String? get savedUri => phase == Phase.done ? _download?.uri : null;

  String get title => switch (job) {
    SendFile(:final file) => 'Sending ${file.name}',
    SendText() => 'Sending text',
    SendTree(:final bundle, :final name, :final entries) =>
      bundle ? 'Sending ${entries.length} files' : 'Sending $name',
    Receive() => offer == null ? 'Receiving' : 'Receiving ${describe(offer!)}',
  };

  static String describe(OfferInfo offer) => switch (offer.kind) {
    OfferKind.files => '${offer.count} files',
    OfferKind.folder => '${offer.name}/',
    _ => offer.name,
  };

  bool get _tree =>
      offer?.kind == OfferKind.folder || offer?.kind == OfferKind.files;

  void start() {
    final running = switch (job) {
      SendFile(:final file) => engine.sendFile(
        code: code,
        name: file.name,
        size: file.size,
        open: () => _descriptor(() => platform.openRead(file.uri)),
      ),
      SendText(:final text) => engine.sendText(code: code, text: text),
      SendTree() => null,
      Receive() => engine.receive(code: code),
    };
    if (running == null) {
      unawaited(_startTree(job as SendTree));
      return;
    }
    _listen(running);
  }

  Future<void> _startTree(SendTree tree) async {
    final cacheDir = await platform.cacheDir();
    if (_disposed || ended) return;
    _listen(
      engine.sendFiles(
        code: code,
        name: tree.name,
        bundle: tree.bundle,
        files: [
          for (final entry in tree.entries)
            (path: entry.path, size: entry.file.size),
        ],
        dirs: tree.dirs,
        open: (index) =>
            _descriptor(() => platform.openRead(tree.entries[index].file.uri)),
        cacheDir: cacheDir,
      ),
    );
  }

  void _listen(Running running) {
    _running = running;
    _updates = running.updates.listen(_update);
    unawaited(platform.transferStarted(title));
  }

  void _update(Update update) {
    if (ended) return;
    stage = update.stage;
    switch (update.stage) {
      case Stage.code:
        shownCode = update.detail;
      case Stage.notice:
        notice = update.detail;
      case Stage.paired:
        break;
      case Stage.hashing || Stage.moving || Stage.awaitingAck:
        phase = Phase.running;
        bytes = update.bytes.toInt();
        total = update.total.toInt();
        if (update.stage == Stage.moving) _measure();
        _progress();
      case Stage.connected:
        phase = Phase.running;
        route = update.detail;
        bytes = 0;
        total = update.total.toInt();
        _moving = clock();
        _sample = (at: _moving!, bytes: 0);
      case Stage.offer:
        offer = _running!.offer();
        if (offer?.kind == OfferKind.text) {
          _end(Phase.done, 'Text received');
        } else {
          phase = Phase.offered;
        }
      case Stage.done:
        if (job is! Receive || _download != null || _accepted) {
          _end(Phase.done, _finished());
        }
      case Stage.failed:
        error = update.detail;
        _end(Phase.failed, update.detail);
    }
    _changed();
  }

  String _finished() => switch (job) {
    SendFile(:final file) => 'Sent ${file.name}',
    SendText() => 'Text sent',
    SendTree() => title.replaceFirst('Sending', 'Sent'),
    Receive() => 'Saved ${describe(offer!)} to Download/wyrmyon',
  };

  void _progress() {
    final now = DateTime.now();
    if (_told != null && now.difference(_told!) < const Duration(seconds: 1)) {
      return;
    }
    _told = now;
    unawaited(
      platform.transferProgress(
        title,
        '${formatBytes(bytes)} of ${formatBytes(total)}',
        bytes,
        total,
      ),
    );
  }

  DateTime? _moving;
  Duration? took;
  double? speed;
  ({DateTime at, int bytes})? _sample;

  void _measure() {
    final now = clock();
    final sample = _sample;
    if (sample == null) return;
    final elapsed = now.difference(sample.at);
    if (elapsed < const Duration(seconds: 1)) return;
    speed = (bytes - sample.bytes) / (elapsed.inMicroseconds / 1e6);
    _sample = (at: now, bytes: bytes);
  }

  bool get verified =>
      phase == Phase.done && job is! SendText && offer?.kind != OfferKind.text;

  List<({String path, bool done, double? progress})> get files {
    if (job case SendTree(:final entries)) {
      final fraction = phase == Phase.done
          ? 1.0
          : total > 0
          ? bytes / total
          : 0.0;
      final size = entries.fold<int>(0, (sum, e) => sum + e.file.size);
      final moved = fraction * size;
      var before = 0;
      return [
        for (final entry in entries)
          () {
            final start = before;
            before += entry.file.size;
            final done = size == 0 || before <= moved;
            final length = entry.file.size;
            return (
              path: entry.path,
              done: done,
              progress: done
                  ? 1.0
                  : length == 0
                  ? 0.0
                  : ((moved - start) / length).clamp(0.0, 1.0),
            );
          }(),
      ];
    }
    return [
      for (final (index, path) in _paths.indexed)
        (
          path: path,
          done: phase == Phase.done || index < _paths.length - 1,
          progress: null,
        ),
    ];
  }

  final _paths = <String>[];

  void _end(Phase ended, String? told) {
    phase = ended;
    if (ended == Phase.done && _moving != null) {
      took = clock().difference(_moving!);
    }
    final download = _download;
    if (download != null) {
      unawaited(
        platform.finishDownload(download.uri, kept: ended == Phase.done),
      );
    }
    for (final uri in _downloads) {
      unawaited(platform.finishDownload(uri, kept: ended == Phase.done));
    }
    unawaited(platform.transferEnded(title, told));
    unawaited(_updates?.cancel());
  }

  bool _accepted = false;

  int get saved => _downloads.length;

  List<String> get savedUris => List.unmodifiable(_downloads);

  Future<void> accept() async {
    if (phase != Phase.offered) return;
    phase = Phase.running;
    _accepted = true;
    _changed();
    if (_tree) return _acceptTree();
    try {
      final name = offer!.name;
      final cache = await platform.cacheDir();
      final download = await platform.createDownload(name);
      if (ended) {
        await platform.finishDownload(download.uri, kept: false);
      } else {
        _download = download;
      }
      await _running!.accept(download.fd, '$cache/received');
    } on Object catch (e) {
      if (ended) return;
      error = '$e';
      if (_download == null) await _running!.refuse();
      if (ended) return;
      _end(Phase.failed, error);
      _changed();
    }
  }

  Future<void> _acceptTree() async {
    final base = offer!.kind == OfferKind.folder ? '${offer!.name}/' : '';
    try {
      final cache = await platform.cacheDir();
      await _running!.acceptFiles(
        (path) => _descriptor(() async {
          if (ended) return -1;
          final download = await platform.createDownload('$base$path');
          if (ended) {
            await platform.finishDownload(download.uri, kept: false);
          } else {
            _downloads.add(download.uri);
            _paths.add(path);
            _changed();
          }
          return download.fd;
        }),
        '$cache/received',
      );
    } on Object catch (e) {
      if (ended) return;
      error = '$e';
      _end(Phase.failed, error);
      _changed();
    }
  }

  Future<void> decline() async {
    if (phase != Phase.offered) return;
    _end(Phase.cancelled, null);
    _changed();
    await _running!.refuse();
  }

  void cancel() {
    if (ended) return;
    final answered = phase == Phase.running;
    _running?.cancel();
    _end(Phase.cancelled, null);
    if (!answered) unawaited(_running?.refuse());
    _changed();
  }

  Future<int> _descriptor(Future<int> Function() open) async {
    try {
      return await open();
    } on Object {
      return -1;
    }
  }

  void _changed() {
    if (!_disposed) notifyListeners();
  }

  @override
  void dispose() {
    cancel();
    _disposed = true;
    super.dispose();
  }
}
