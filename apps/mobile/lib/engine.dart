import 'src/rust/api/bridge.dart' as bridge;
import 'src/rust/api/types.dart';
import 'src/rust/api/types.dart' as rust;

export 'src/rust/api/types.dart'
    show Direction, OfferInfo, OfferKind, Scanned, Stage, Update;

abstract class Running {
  Stream<Update> get updates;
  void cancel();
  OfferInfo? offer();
  Future<void> accept(int fd, String cacheDir);
  Future<void> acceptFiles(
    Future<int> Function(String path) open,
    String cacheDir,
  );
  Future<void> refuse();
}

typedef TreeFile = ({String path, int size});

abstract class Engine {
  Scanned? scan(String text);
  List<String> completions(String typed);
  Running sendFile({
    String? code,
    required String name,
    required int size,
    required Future<int> Function() open,
  });
  Running sendText({String? code, required String text});
  Running sendFiles({
    String? code,
    required String name,
    required bool bundle,
    required List<TreeFile> files,
    required List<String> dirs,
    required Future<int> Function(int index) open,
    required String cacheDir,
  });
  Running receive({String? code});
}

class RustEngine implements Engine {
  RustEngine() : _settings = rust.defaultSettings();

  final Settings _settings;

  @override
  Scanned? scan(String text) {
    try {
      return rust.scan(text: text);
    } on Object {
      return null;
    }
  }

  @override
  List<String> completions(String typed) => rust.completions(typed: typed);

  @override
  Running sendFile({
    String? code,
    required String name,
    required int size,
    required Future<int> Function() open,
  }) {
    final transfer = bridge.Transfer();
    return _Running(
      transfer,
      bridge.sendFile(
        settings: _settings,
        code: code,
        name: name,
        size: BigInt.from(size),
        open: open,
        transfer: transfer,
      ),
    );
  }

  @override
  Running sendText({String? code, required String text}) {
    final transfer = bridge.Transfer();
    return _Running(
      transfer,
      bridge.sendText(
        settings: _settings,
        code: code,
        text: text,
        transfer: transfer,
      ),
    );
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
    final transfer = bridge.Transfer();
    return _Running(
      transfer,
      bridge.sendFiles(
        settings: _settings,
        code: code,
        name: name,
        bundle: bundle,
        files: [
          for (final file in files)
            bridge.Picked(path: file.path, size: BigInt.from(file.size)),
        ],
        dirs: dirs,
        open: open,
        cacheDir: cacheDir,
        transfer: transfer,
      ),
    );
  }

  @override
  Running receive({String? code}) {
    final transfer = bridge.Transfer();
    return _Running(
      transfer,
      bridge.receive(settings: _settings, code: code, transfer: transfer),
    );
  }
}

class _Running implements Running {
  _Running(this._transfer, this.updates);

  final bridge.Transfer _transfer;

  @override
  final Stream<Update> updates;

  @override
  void cancel() => _transfer.cancel();

  @override
  OfferInfo? offer() => _transfer.offer();

  @override
  Future<void> accept(int fd, String cacheDir) =>
      bridge.accept(transfer: _transfer, fd: fd, cacheDir: cacheDir);

  @override
  Future<void> acceptFiles(
    Future<int> Function(String path) open,
    String cacheDir,
  ) => bridge.acceptFiles(transfer: _transfer, open: open, cacheDir: cacheDir);

  @override
  Future<void> refuse() => bridge.refuse(transfer: _transfer);
}
