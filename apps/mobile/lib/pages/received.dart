import 'dart:async';

import 'package:flutter/material.dart';

import '../app.dart';
import '../format.dart';
import '../platform.dart';

class ReceivedPage extends StatefulWidget {
  const ReceivedPage({super.key});

  static Route<void> route() =>
      MaterialPageRoute(builder: (_) => const ReceivedPage());

  @override
  State<ReceivedPage> createState() => _ReceivedPageState();
}

class _ReceivedPageState extends State<ReceivedPage> {
  List<Download>? _all;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_all == null) unawaited(_load());
  }

  Future<void> _load() async {
    final platform = Services.of(context).platform;
    final all = await platform.recentDownloads(1000);
    if (mounted) setState(() => _all = all);
  }

  @override
  Widget build(BuildContext context) {
    final all = _all;
    return Scaffold(
      appBar: AppBar(
        title: const Text('Received'),
        actions: [
          IconButton(
            tooltip: 'Show in folder',
            icon: const Icon(Icons.folder_open_outlined),
            onPressed: () => Services.of(context).platform.showDownloads(),
          ),
        ],
      ),
      body: all == null
          ? const Center(child: CircularProgressIndicator())
          : ListView(children: [for (final d in all) ReceivedTile(d)]),
    );
  }
}

class ReceivedTile extends StatelessWidget {
  const ReceivedTile(this.download, {super.key});

  final Download download;

  @override
  Widget build(BuildContext context) {
    final platform = Services.of(context).platform;
    final messenger = ScaffoldMessenger.of(context);
    return ListTile(
      leading: const Icon(Icons.insert_drive_file_outlined),
      title: Text(download.name, overflow: TextOverflow.ellipsis),
      subtitle: Text(
        '${download.folder.isEmpty ? '' : '${download.folder}/ · '}'
        '${formatAge(download.modified, DateTime.now())} · '
        '${formatBytes(download.size)}',
      ),
      onTap: () async {
        if (!await platform.openDownload(download.uri)) {
          messenger.showSnackBar(
            const SnackBar(content: Text('No app can open this file')),
          );
        }
      },
      trailing: IconButton(
        tooltip: 'Share',
        icon: const Icon(Icons.share),
        onPressed: () => platform.shareDownload(download.uri),
      ),
    );
  }
}
