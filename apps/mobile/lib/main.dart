import 'package:flutter/material.dart';

import 'src/rust/frb_generated.dart';

Future<void> main() async {
  await RustLib.init();
  runApp(const WyrmyonApp());
}

class WyrmyonApp extends StatelessWidget {
  const WyrmyonApp({super.key});

  @override
  Widget build(BuildContext context) {
    const seed = Color(0xFF2E7D5B);
    return MaterialApp(
      title: 'wyrmyon',
      theme: ThemeData(colorSchemeSeed: seed),
      darkTheme: ThemeData(colorSchemeSeed: seed, brightness: Brightness.dark),
      home: const Home(),
    );
  }
}

class Home extends StatelessWidget {
  const Home({super.key});

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text('wyrmyon')),
      body: const Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          spacing: 16,
          children: [
            FilledButton(onPressed: null, child: Text('Send')),
            OutlinedButton(onPressed: null, child: Text('Receive')),
          ],
        ),
      ),
    );
  }
}
