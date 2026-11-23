import 'dart:async';

import 'package:flutter/material.dart';

import '../app.dart';
import '../platform.dart';
import '../transfer.dart';
import 'received.dart';
import 'scan.dart';
import 'send.dart';
import 'settings.dart';
import 'transfer.dart';

class HomePage extends StatefulWidget {
  const HomePage({super.key});

  @override
  State<HomePage> createState() => _HomePageState();
}

class _HomePageState extends State<HomePage>
    with WidgetsBindingObserver, RouteAware {
  List<Download>? _recent;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    returning.subscribe(this, ModalRoute.of(context)!);
    if (_recent == null) unawaited(_reload());
  }

  @override
  void didPopNext() => unawaited(_reload());

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    returning.unsubscribe(this);
    super.dispose();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state == AppLifecycleState.resumed) unawaited(_reload());
  }

  Future<void> _reload() async {
    final platform = Services.of(context).platform;
    try {
      final recent = await platform.recentDownloads(10);
      if (mounted) setState(() => _recent = recent);
    } on Object {
      if (mounted) setState(() => _recent = const []);
    }
  }

  Future<void> _send() async {
    final platform = Services.of(context).platform;
    final navigator = Navigator.of(context);
    final files = await platform.pickFiles();
    if (files.isEmpty) return;
    await navigator.push(SendPage.route(files: files));
  }

  Future<void> _receive() async {
    final navigator = Navigator.of(context);
    final code = await navigator.push(ScanPage.route(Purpose.receive));
    if (code == null) return;
    await navigator.push(TransferPage.route(const Receive(), code: code));
  }

  @override
  Widget build(BuildContext context) {
    final recent = _recent;
    return Scaffold(
      appBar: AppBar(
        title: const Text('wyrmyon'),
        actions: [
          IconButton(
            tooltip: 'Settings',
            icon: const Icon(Icons.settings_outlined),
            onPressed: () => Navigator.of(context).push(SettingsPage.route()),
          ),
        ],
      ),
      body: ListView(
        padding: const EdgeInsets.all(16),
        children: [
          _Action(
            icon: Icons.upload,
            title: 'Send files',
            subtitle: 'Pick files, then scan a code',
            onTap: _send,
          ),
          const SizedBox(height: 12),
          _Action(
            icon: Icons.download,
            title: 'Receive',
            subtitle: 'Scan or type a code',
            onTap: _receive,
          ),
          const SizedBox(height: 24),
          Row(
            children: [
              Expanded(
                child: Text(
                  'Latest received',
                  style: Theme.of(context).textTheme.titleMedium,
                ),
              ),
              if (recent != null && recent.isNotEmpty)
                TextButton(
                  onPressed: () =>
                      Navigator.of(context).push(ReceivedPage.route()),
                  child: const Text('All ›'),
                ),
            ],
          ),
          const SizedBox(height: 8),
          if (recent == null)
            const Center(child: CircularProgressIndicator())
          else if (recent.isEmpty)
            const ListTile(
              title: Text('Nothing received yet'),
              subtitle: Text('Received files land in Download/wyrmyon'),
            )
          else
            for (final download in recent) ReceivedTile(download),
        ],
      ),
    );
  }
}

class _Action extends StatelessWidget {
  const _Action({
    required this.icon,
    required this.title,
    required this.subtitle,
    required this.onTap,
  });

  final IconData icon;
  final String title;
  final String subtitle;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return Card.filled(
      clipBehavior: Clip.antiAlias,
      child: ListTile(
        contentPadding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
        leading: Icon(icon, size: 32),
        title: Text(title),
        subtitle: Text(subtitle),
        onTap: onTap,
      ),
    );
  }
}
