import 'dart:async';

import 'package:flutter/material.dart';

import 'engine.dart';
import 'pages/home.dart';
import 'pages/send.dart';
import 'pages/transfer.dart';
import 'platform.dart';
import 'settings.dart';
import 'transfer.dart';
import 'ui.dart';

final returning = RouteObserver<ModalRoute<void>>();

class Services extends InheritedWidget {
  const Services({
    super.key,
    required this.engine,
    required this.platform,
    required this.settings,
    required super.child,
  });

  final Engine engine;
  final Platform platform;
  final SettingsStore settings;

  static Services of(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<Services>()!;

  @override
  bool updateShouldNotify(Services oldWidget) =>
      engine != oldWidget.engine ||
      platform != oldWidget.platform ||
      settings != oldWidget.settings;
}

class WyrmyonApp extends StatefulWidget {
  const WyrmyonApp({
    super.key,
    required this.engine,
    required this.platform,
    required this.settings,
  });

  final Engine engine;
  final Platform platform;
  final SettingsStore settings;

  @override
  State<WyrmyonApp> createState() => _WyrmyonAppState();
}

class _WyrmyonAppState extends State<WyrmyonApp> {
  final _navigator = GlobalKey<NavigatorState>();
  final _messenger = GlobalKey<ScaffoldMessengerState>();
  StreamSubscription<Shared>? _shared;

  @override
  void initState() {
    super.initState();
    _shared = widget.platform.shared.listen(_open);
    unawaited(
      widget.platform.takeShared().then((shared) {
        if (shared != null) _open(shared);
      }),
    );
  }

  @override
  void dispose() {
    unawaited(_shared?.cancel());
    super.dispose();
  }

  void _open(Shared shared) {
    final navigator = _navigator.currentState;
    if (navigator == null) return;
    switch (shared) {
      case SharedFiles(:final files):
        unawaited(navigator.push(SendPage.route(files: files)));
      case SharedText(:final text):
        unawaited(navigator.push(SendPage.route(text: text)));
      case SharedLink(:final link):
        final scanned = widget.engine.scan(link);
        if (scanned == null) {
          _tell('This link holds no wormhole code');
        } else if (scanned.direction == Direction.send) {
          unawaited(navigator.push(SendPage.route(code: scanned.code)));
        } else {
          unawaited(
            navigator.push(
              TransferPage.route(const Receive(), code: scanned.code),
            ),
          );
        }
      case SharedError(:final error):
        _tell(error);
    }
  }

  void _tell(String message) {
    _messenger.currentState?.showSnackBar(SnackBar(content: Text(message)));
  }

  @override
  Widget build(BuildContext context) {
    return Services(
      engine: widget.engine,
      platform: widget.platform,
      settings: widget.settings,
      child: MaterialApp(
        title: 'wyrmyon',
        navigatorKey: _navigator,
        scaffoldMessengerKey: _messenger,
        navigatorObservers: [returning],
        theme: wyrmyonTheme(),
        debugShowCheckedModeBanner: false,
        home: const HomePage(),
      ),
    );
  }
}
