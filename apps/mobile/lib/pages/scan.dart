import 'package:flutter/material.dart';
import 'package:mobile_scanner/mobile_scanner.dart';

import '../app.dart';
import '../engine.dart';
import 'type_code.dart';

enum Purpose { send, receive }

class ScanPage extends StatefulWidget {
  const ScanPage({super.key, required this.purpose});

  final Purpose purpose;

  static Route<String> route(Purpose purpose) =>
      MaterialPageRoute(builder: (_) => ScanPage(purpose: purpose));

  @override
  State<ScanPage> createState() => _ScanPageState();
}

class _ScanPageState extends State<ScanPage> {
  String? _problem;
  bool _done = false;

  void _detect(BarcodeCapture capture) {
    if (_done) return;
    final engine = Services.of(context).engine;
    for (final barcode in capture.barcodes) {
      final text = barcode.rawValue;
      if (text == null) continue;
      final scanned = engine.scan(text);
      final wanted = widget.purpose == Purpose.send
          ? Direction.send
          : Direction.receive;
      if (scanned == null) {
        setState(() => _problem = 'This is not a wormhole code');
      } else if (scanned.direction != wanted) {
        setState(
          () => _problem = wanted == Direction.send
              ? 'This code sends to the phone: use Receive'
              : 'This code waits for files: use Send',
        );
      } else {
        _done = true;
        Navigator.of(context).pop(scanned.code);
        return;
      }
    }
  }

  Future<void> _type() async {
    final navigator = Navigator.of(context);
    final code = await navigator.push(TypeCodePage.route());
    if (code != null) navigator.pop(code);
  }

  @override
  Widget build(BuildContext context) {
    final command = widget.purpose == Purpose.send
        ? 'wyrm receive --new --qr'
        : 'wyrm send --qr FILE';
    final scheme = Theme.of(context).colorScheme;
    return Scaffold(
      appBar: AppBar(title: const Text("Scan the computer's code")),
      body: Column(
        children: [
          Expanded(
            child: MobileScanner(
              onDetect: _detect,
              errorBuilder: (context, error) => Center(
                child: Padding(
                  padding: const EdgeInsets.all(24),
                  child: Text(
                    'The camera is not available: '
                    '${error.errorDetails?.message ?? error.errorCode.name}',
                    textAlign: TextAlign.center,
                  ),
                ),
              ),
            ),
          ),
          Padding(
            padding: const EdgeInsets.all(16),
            child: Column(
              spacing: 8,
              children: [
                if (_problem case final problem?)
                  Text(problem, style: TextStyle(color: scheme.error)),
                const Text('On your computer, run'),
                SelectableText(
                  command,
                  style: const TextStyle(fontFamily: 'monospace'),
                ),
                OutlinedButton.icon(
                  onPressed: _type,
                  icon: const Icon(Icons.keyboard),
                  label: const Text('Type the code'),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}
