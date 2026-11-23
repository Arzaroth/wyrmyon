import 'package:flutter/material.dart';

import 'app.dart';
import 'engine.dart';
import 'platform.dart';
import 'src/rust/frb_generated.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await RustLib.init();
  runApp(WyrmyonApp(engine: RustEngine(), platform: AndroidPlatform()));
}
