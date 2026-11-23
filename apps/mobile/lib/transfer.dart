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

class Receive extends Job {
  const Receive();
}

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
  DateTime? _told;
  bool _disposed = false;

  bool get sending => job is! Receive;
  bool get ended =>
      phase == Phase.done || phase == Phase.failed || phase == Phase.cancelled;
  String? get savedUri => phase == Phase.done ? _download?.uri : null;

  String get title => switch (job) {
    SendFile(:final file) => 'Sending ${file.name}',
    SendText() => 'Sending text',
    Receive() => offer == null ? 'Receiving' : 'Receiving ${offer!.name}',
  };

  void start() {
    final running = switch (job) {
      SendFile(:final file) => engine.sendFile(
        code: code,
        name: file.name,
        size: file.size,
        open: () => platform.openRead(file.uri),
      ),
      SendText(:final text) => engine.sendText(code: code, text: text),
      Receive() => engine.receive(code: code),
    };
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
        _progress();
      case Stage.connected:
        phase = Phase.running;
        route = update.detail;
        bytes = 0;
        total = update.total.toInt();
      case Stage.offer:
        offer = _running!.offer();
        if (offer?.kind == OfferKind.text) {
          _end(Phase.done, 'Text received');
        } else {
          phase = Phase.offered;
        }
      case Stage.done:
        if (job is! Receive || _download != null) {
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
    Receive() => 'Saved ${offer?.name ?? ''} to Download/wyrmyon',
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

  void _end(Phase ended, String? told) {
    phase = ended;
    final download = _download;
    if (download != null) {
      unawaited(
        platform.finishDownload(download.uri, kept: ended == Phase.done),
      );
    }
    unawaited(platform.transferEnded(title, told));
    unawaited(_updates?.cancel());
  }

  Future<void> accept() async {
    if (phase != Phase.offered) return;
    phase = Phase.running;
    _changed();
    try {
      final name = offer!.name;
      final cache = await platform.cacheDir();
      final download = await platform.createDownload(name);
      _download = download;
      await _running!.accept(download.fd, '$cache/received');
    } on Object catch (e) {
      if (ended) return;
      error = '$e';
      if (_download == null) await _running!.refuse();
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
    final offered = phase == Phase.offered;
    _running?.cancel();
    _end(Phase.cancelled, null);
    if (offered) unawaited(_running!.refuse());
    _changed();
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
