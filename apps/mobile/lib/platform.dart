import 'dart:async';

import 'package:flutter/services.dart';

class PickedFile {
  const PickedFile({required this.uri, required this.name, required this.size});

  final String uri;
  final String name;
  final int size;

  static PickedFile of(Map<Object?, Object?> map) => PickedFile(
    uri: map['uri']! as String,
    name: map['name']! as String,
    size: (map['size']! as num).toInt(),
  );
}

class Download {
  const Download({
    required this.uri,
    required this.name,
    required this.size,
    required this.modified,
  });

  final String uri;
  final String name;
  final int size;
  final DateTime modified;
}

sealed class Shared {
  const Shared();
}

class SharedFiles extends Shared {
  const SharedFiles(this.files);
  final List<PickedFile> files;
}

class SharedText extends Shared {
  const SharedText(this.text);
  final String text;
}

class SharedLink extends Shared {
  const SharedLink(this.link);
  final String link;
}

class SharedError extends Shared {
  const SharedError(this.error);
  final String error;
}

abstract class Platform {
  Future<Shared?> takeShared();
  Stream<Shared> get shared;
  Future<List<PickedFile>> pickFiles();
  Future<int> openRead(String uri);
  Future<String> cacheDir();
  Future<({String uri, int fd})> createDownload(String name);
  Future<void> finishDownload(String uri, {required bool kept});
  Future<List<Download>> recentDownloads(int limit);
  Future<bool> openDownload(String uri);
  Future<bool> shareDownload(String uri);
  Future<void> transferStarted(String title);
  Future<void> transferProgress(
    String title,
    String text,
    int bytes,
    int total,
  );
  Future<void> transferEnded(String title, String? text);
}

class AndroidPlatform implements Platform {
  AndroidPlatform() {
    _channel.setMethodCallHandler((call) async {
      if (call.method == 'intent') {
        final shared = _shared(call.arguments as Map<Object?, Object?>?);
        if (shared != null) _intents.add(shared);
      }
    });
  }

  static const _channel = MethodChannel('wyrmyon/platform');
  final _intents = StreamController<Shared>.broadcast();

  @override
  Stream<Shared> get shared => _intents.stream;

  @override
  Future<Shared?> takeShared() async =>
      _shared(await _channel.invokeMethod<Map<Object?, Object?>>('takeIntent'));

  static Shared? _shared(Map<Object?, Object?>? map) => switch (map?['kind']) {
    'files' => SharedFiles([
      for (final file in map!['files']! as List<Object?>)
        PickedFile.of(file! as Map<Object?, Object?>),
    ]),
    'text' => SharedText(map!['text']! as String),
    'link' => SharedLink(map!['link']! as String),
    'error' => SharedError(map!['error']! as String),
    _ => null,
  };

  @override
  Future<List<PickedFile>> pickFiles() async {
    final picked = await _channel.invokeMethod<List<Object?>>('pickFiles');
    return [
      for (final file in picked ?? const [])
        PickedFile.of(file! as Map<Object?, Object?>),
    ];
  }

  @override
  Future<int> openRead(String uri) async =>
      await _channel.invokeMethod<int>('openRead', {'uri': uri}) ?? -1;

  @override
  Future<String> cacheDir() async =>
      (await _channel.invokeMethod<String>('cacheDir'))!;

  @override
  Future<({String uri, int fd})> createDownload(String name) async {
    final created = (await _channel.invokeMethod<Map<Object?, Object?>>(
      'createDownload',
      {'name': name},
    ))!;
    return (uri: created['uri']! as String, fd: created['fd']! as int);
  }

  @override
  Future<void> finishDownload(String uri, {required bool kept}) =>
      _channel.invokeMethod('finishDownload', {'uri': uri, 'kept': kept});

  @override
  Future<List<Download>> recentDownloads(int limit) async {
    final found = await _channel.invokeMethod<List<Object?>>(
      'recentDownloads',
      {'limit': limit},
    );
    return [
      for (final item in found ?? const [])
        if (item case final Map<Object?, Object?> map)
          Download(
            uri: map['uri']! as String,
            name: map['name']! as String,
            size: (map['size']! as num).toInt(),
            modified: DateTime.fromMillisecondsSinceEpoch(
              (map['modified']! as num).toInt(),
            ),
          ),
    ];
  }

  @override
  Future<bool> openDownload(String uri) async =>
      await _channel.invokeMethod<bool>('openDownload', {'uri': uri}) ?? false;

  @override
  Future<bool> shareDownload(String uri) async =>
      await _channel.invokeMethod<bool>('shareDownload', {'uri': uri}) ?? false;

  @override
  Future<void> transferStarted(String title) =>
      _channel.invokeMethod('transferStarted', {'title': title});

  @override
  Future<void> transferProgress(
    String title,
    String text,
    int bytes,
    int total,
  ) => _channel.invokeMethod('transferProgress', {
    'title': title,
    'text': text,
    'bytes': bytes,
    'total': total,
  });

  @override
  Future<void> transferEnded(String title, String? text) =>
      _channel.invokeMethod('transferEnded', {'title': title, 'text': text});
}
