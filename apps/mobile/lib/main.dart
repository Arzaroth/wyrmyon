import 'package:flutter/material.dart';

import 'app.dart';
import 'engine.dart';
import 'platform.dart';
import 'settings.dart';
import 'src/rust/frb_generated.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await RustLib.init();
  final engine = RustEngine();
  final settings = PreferencesStore();
  final kept = await settings.load();
  if (kept != null && engine.check(kept) == null) engine.settings = kept;
  runApp(
    WyrmyonApp(engine: engine, platform: AndroidPlatform(), settings: settings),
  );
}
