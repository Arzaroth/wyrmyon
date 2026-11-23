import 'package:flutter/material.dart';

import '../app.dart';
import '../format.dart';
import '../platform.dart';
import '../transfer.dart';
import 'scan.dart';
import 'transfer.dart';
import 'type_code.dart';

class SendPage extends StatefulWidget {
  const SendPage({super.key, this.files = const [], this.text, this.code});

  final List<PickedFile> files;
  final String? text;
  final String? code;

  static Route<void> route({
    List<PickedFile> files = const [],
    String? text,
    String? code,
  }) => MaterialPageRoute(
    builder: (_) => SendPage(files: files, text: text, code: code),
  );

  @override
  State<SendPage> createState() => _SendPageState();
}

class _SendPageState extends State<SendPage> {
  late final List<PickedFile> _files = [...widget.files];
  final List<PickedFolder> _folders = [];
  late String? _text = widget.text;

  int get _count => _files.length + _folders.length;

  String? get _clash {
    final seen = <String>{};
    for (final name in [
      for (final file in _files) file.name,
      for (final folder in _folders) folder.name,
    ]) {
      if (!seen.add(name)) return name;
    }
    return null;
  }

  Job? get _job {
    if (_text != null) return _count == 0 ? SendText(_text!) : null;
    if (_count == 0 || _clash != null) return null;
    if (_files.length == 1 && _folders.isEmpty) return SendFile(_files.single);
    if (_folders.length == 1 && _files.isEmpty) {
      final folder = _folders.single;
      return SendTree(
        name: folder.name,
        bundle: false,
        entries: [
          for (final file in folder.files) (path: file.name, file: file),
        ],
        dirs: folder.dirs,
      );
    }
    return SendTree(
      name: '$_count items',
      bundle: true,
      entries: [
        for (final file in _files) (path: file.name, file: file),
        for (final folder in _folders)
          for (final file in folder.files)
            (path: '${folder.name}/${file.name}', file: file),
      ],
      dirs: [
        for (final folder in _folders) ...[
          if (folder.files.isEmpty && folder.dirs.isEmpty) folder.name,
          for (final dir in folder.dirs) '${folder.name}/$dir',
        ],
      ],
    );
  }

  Future<void> _add() async {
    final picked = await Services.of(context).platform.pickFiles();
    if (picked.isNotEmpty) setState(() => _files.addAll(picked));
  }

  Future<void> _addFolder() async {
    final picked = await Services.of(context).platform.pickFolder();
    if (picked != null) setState(() => _folders.add(picked));
  }

  Future<void> _start(
    Future<String?> Function() code, {
    bool show = false,
  }) async {
    final job = _job!;
    final navigator = Navigator.of(context);
    final chosen = show ? null : await code();
    if (!show && chosen == null) return;
    await navigator.pushReplacement(TransferPage.route(job, code: chosen));
  }

  @override
  Widget build(BuildContext context) {
    final job = _job;
    final total =
        _files.fold<int>(0, (sum, file) => sum + file.size) +
        _folders.fold<int>(0, (sum, folder) => sum + folder.size);
    final code = widget.code;
    final unknown = [
      ..._files,
      for (final folder in _folders) ...folder.files,
    ].where((file) => file.size < 0).toList();
    final clash = _clash;
    final ready = job != null && unknown.isEmpty;
    return Scaffold(
      appBar: AppBar(
        title: const Text('Send'),
        bottom: PreferredSize(
          preferredSize: const Size.fromHeight(24),
          child: Padding(
            padding: const EdgeInsets.only(bottom: 8),
            child: Text(
              _text != null
                  ? 'Text'
                  : '$_count '
                        '${_count == 1 ? 'item' : 'items'} · '
                        '${formatBytes(total)}',
            ),
          ),
        ),
      ),
      body: ListView(
        padding: const EdgeInsets.all(16),
        children: [
          if (_text case final text?)
            Card.outlined(
              child: ListTile(
                leading: const Icon(Icons.notes),
                title: Text(text, maxLines: 4, overflow: TextOverflow.ellipsis),
                trailing: IconButton(
                  tooltip: 'Remove',
                  icon: const Icon(Icons.close),
                  onPressed: () => setState(() => _text = null),
                ),
              ),
            ),
          for (final file in _files)
            ListTile(
              leading: const Icon(Icons.insert_drive_file_outlined),
              title: Text(file.name, overflow: TextOverflow.ellipsis),
              subtitle: Text(
                file.size < 0 ? 'size unknown' : formatBytes(file.size),
              ),
              trailing: IconButton(
                tooltip: 'Remove',
                icon: const Icon(Icons.close),
                onPressed: () => setState(() => _files.remove(file)),
              ),
            ),
          for (final folder in _folders)
            ListTile(
              leading: const Icon(Icons.folder_outlined),
              title: Text('${folder.name}/', overflow: TextOverflow.ellipsis),
              subtitle: Text(
                '${folder.files.length} files · ${formatBytes(folder.size)}',
              ),
              trailing: IconButton(
                tooltip: 'Remove',
                icon: const Icon(Icons.close),
                onPressed: () => setState(() => _folders.remove(folder)),
              ),
            ),
          const SizedBox(height: 8),
          Row(
            spacing: 8,
            children: [
              OutlinedButton.icon(
                onPressed: _add,
                icon: const Icon(Icons.add),
                label: const Text('Files'),
              ),
              OutlinedButton.icon(
                onPressed: _addFolder,
                icon: const Icon(Icons.create_new_folder_outlined),
                label: const Text('Folder'),
              ),
            ],
          ),
          if (_text != null && _count > 0)
            const _Hint('Text is sent on its own: remove it or the files'),
          if (clash != null) _Hint('Two items are named $clash'),
          if (unknown.isNotEmpty)
            _Hint(
              'The size of ${unknown.first.name} is unknown, so it cannot be sent',
            ),
        ],
      ),
      bottomNavigationBar: SafeArea(
        child: Padding(
          padding: const EdgeInsets.all(16),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            spacing: 8,
            children: code != null
                ? [
                    FilledButton.icon(
                      onPressed: ready ? () => _start(() async => code) : null,
                      icon: const Icon(Icons.send),
                      label: Text('Send to $code'),
                    ),
                  ]
                : [
                    FilledButton.icon(
                      onPressed: ready
                          ? () => _start(
                              () => Navigator.of(
                                context,
                              ).push(ScanPage.route(Purpose.send)),
                            )
                          : null,
                      icon: const Icon(Icons.qr_code_scanner),
                      label: const Text('Scan code to send'),
                    ),
                    Row(
                      spacing: 8,
                      children: [
                        Expanded(
                          child: OutlinedButton(
                            onPressed: ready
                                ? () => _start(
                                    () => Navigator.of(
                                      context,
                                    ).push(TypeCodePage.route()),
                                  )
                                : null,
                            child: const Text('Type a code'),
                          ),
                        ),
                        Expanded(
                          child: OutlinedButton(
                            onPressed: ready
                                ? () => _start(() async => null, show: true)
                                : null,
                            child: const Text('Show a code'),
                          ),
                        ),
                      ],
                    ),
                  ],
          ),
        ),
      ),
    );
  }
}

class _Hint extends StatelessWidget {
  const _Hint(this.text);

  final String text;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(top: 16),
      child: Text(
        text,
        style: TextStyle(color: Theme.of(context).colorScheme.error),
      ),
    );
  }
}
