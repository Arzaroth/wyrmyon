import 'dart:async';

import 'package:flutter/material.dart';

import '../app.dart';
import '../platform.dart';
import '../transfer.dart';
import '../ui.dart';
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
    return Screen(
      body: ListView(
        padding: const EdgeInsets.fromLTRB(24, 8, 24, 24),
        children: [
          Align(
            alignment: Alignment.centerRight,
            child: IconButton(
              tooltip: 'Settings',
              color: Palette.dim,
              icon: const Icon(Icons.settings_outlined),
              onPressed: () => Navigator.of(context).push(SettingsPage.route()),
            ),
          ),
          const _Hero(),
          const SizedBox(height: 28),
          _Big(
            primary: true,
            icon: Icons.arrow_upward,
            title: 'Send files',
            subtitle: 'Pick files, then scan a code',
            onTap: _send,
          ),
          const SizedBox(height: 14),
          _Big(
            primary: false,
            icon: Icons.arrow_downward,
            title: 'Receive',
            subtitle: 'Scan or type a code',
            onTap: _receive,
          ),
          const SizedBox(height: 24),
          Row(
            children: [
              const Expanded(child: Label('Latest received')),
              if (recent != null && recent.isNotEmpty)
                Ghost(
                  'All ›',
                  onPressed: () =>
                      Navigator.of(context).push(ReceivedPage.route()),
                ),
            ],
          ),
          const SizedBox(height: 6),
          if (recent == null)
            const Padding(
              padding: EdgeInsets.all(24),
              child: Center(child: CircularProgressIndicator()),
            )
          else if (recent.isEmpty)
            const Padding(
              padding: EdgeInsets.symmetric(vertical: 12),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                spacing: 4,
                children: [
                  Text('Nothing received yet', style: TextStyle(fontSize: 16)),
                  Text(
                    'Received files land in Download/wyrmyon',
                    style: subStyle,
                  ),
                ],
              ),
            )
          else
            for (final download in recent) ReceivedTile(download),
        ],
      ),
    );
  }
}

class _Hero extends StatelessWidget {
  const _Hero();

  @override
  Widget build(BuildContext context) {
    return Column(
      children: [
        DecoratedBox(
          decoration: BoxDecoration(
            shape: BoxShape.circle,
            boxShadow: [
              BoxShadow(
                color: Palette.cyan.withValues(alpha: 0.45),
                blurRadius: 36,
                spreadRadius: 2,
              ),
            ],
          ),
          child: Image.asset(
            'assets/logo.png',
            width: 140,
            height: 140,
            filterQuality: FilterQuality.none,
          ),
        ),
        const SizedBox(height: 14),
        const Text(
          'wyrmyon',
          style: TextStyle(
            fontSize: 30,
            fontWeight: FontWeight.w700,
            letterSpacing: 0.3,
          ),
        ),
      ],
    );
  }
}

class _Big extends StatelessWidget {
  const _Big({
    required this.primary,
    required this.icon,
    required this.title,
    required this.subtitle,
    required this.onTap,
  });

  final bool primary;
  final IconData icon;
  final String title;
  final String subtitle;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return DecoratedBox(
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(26),
        gradient: primary
            ? const LinearGradient(colors: [Palette.blue, Palette.sky])
            : null,
        border: primary ? null : Border.all(color: Palette.line, width: 1.5),
        boxShadow: primary
            ? [
                BoxShadow(
                  color: Palette.blue.withValues(alpha: 0.35),
                  blurRadius: 22,
                  offset: const Offset(0, 8),
                ),
              ]
            : null,
      ),
      child: Material(
        type: MaterialType.transparency,
        child: InkWell(
          borderRadius: BorderRadius.circular(26),
          onTap: onTap,
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 22),
            child: Row(
              spacing: 18,
              children: [
                Tile(
                  icon,
                  size: 58,
                  color: primary
                      ? Colors.white.withValues(alpha: 0.16)
                      : Palette.tile,
                  iconColor: primary ? Colors.white : Palette.cyan,
                ),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    spacing: 3,
                    children: [
                      Text(
                        title,
                        style: const TextStyle(
                          fontSize: 21,
                          fontWeight: FontWeight.w500,
                          color: Colors.white,
                        ),
                      ),
                      Text(
                        subtitle,
                        style: TextStyle(
                          fontSize: 14.5,
                          color: primary
                              ? Colors.white.withValues(alpha: 0.82)
                              : Palette.dim,
                        ),
                      ),
                    ],
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
