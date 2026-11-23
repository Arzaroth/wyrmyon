import 'package:flutter/material.dart';

import '../app.dart';
import '../ui.dart';
import 'scan.dart';

class TypeCodePage extends StatefulWidget {
  const TypeCodePage({
    super.key,
    this.purpose = Purpose.send,
    this.fromScanner = false,
  });

  final Purpose purpose;
  final bool fromScanner;

  static Route<String> route({
    Purpose purpose = Purpose.send,
    bool fromScanner = false,
  }) => MaterialPageRoute(
    builder: (_) => TypeCodePage(purpose: purpose, fromScanner: fromScanner),
  );

  @override
  State<TypeCodePage> createState() => _TypeCodePageState();
}

class _TypeCodePageState extends State<TypeCodePage> {
  final _typed = TextEditingController();

  @override
  void dispose() {
    _typed.dispose();
    super.dispose();
  }

  void _complete(String word) {
    _typed.value = TextEditingValue(
      text: word,
      selection: TextSelection.collapsed(offset: word.length),
    );
  }

  void _connect() {
    final code = Services.of(context).engine.scan(_typed.text);
    if (code != null) Navigator.of(context).pop(code.code);
  }

  Future<void> _scan() async {
    final navigator = Navigator.of(context);
    if (widget.fromScanner) {
      navigator.pop();
      return;
    }
    final code = await navigator.push(ScanPage.route(widget.purpose));
    if (code != null) navigator.pop(code);
  }

  @override
  Widget build(BuildContext context) {
    final engine = Services.of(context).engine;
    return ListenableBuilder(
      listenable: _typed,
      builder: (context, _) {
        final suggestions = engine.completions(_typed.text);
        final valid = engine.scan(_typed.text) != null;
        return Screen(
          title: 'Enter code',
          body: ListView(
            padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 8),
            children: [
              TextField(
                controller: _typed,
                autofocus: true,
                autocorrect: false,
                enableSuggestions: false,
                textAlign: TextAlign.center,
                cursorColor: Palette.cyan,
                keyboardType: TextInputType.visiblePassword,
                style: const TextStyle(
                  fontFamily: mono,
                  fontSize: 24,
                  color: Palette.fg,
                ),
                decoration: const InputDecoration(
                  hintText: '7-guitarist-revenge',
                  hintStyle: TextStyle(color: Palette.line),
                  contentPadding: EdgeInsets.symmetric(
                    horizontal: 16,
                    vertical: 20,
                  ),
                ),
                onSubmitted: (_) => _connect(),
              ),
              const SizedBox(height: 20),
              if (suggestions.isNotEmpty) ...[
                const Label('Suggestions'),
                const SizedBox(height: 10),
                Wrap(
                  spacing: 8,
                  runSpacing: 8,
                  children: [
                    for (final suggestion in suggestions.take(12))
                      Material(
                        color: Palette.tile,
                        borderRadius: BorderRadius.circular(12),
                        child: InkWell(
                          borderRadius: BorderRadius.circular(12),
                          onTap: () => _complete(suggestion),
                          child: Padding(
                            padding: const EdgeInsets.symmetric(
                              horizontal: 14,
                              vertical: 9,
                            ),
                            child: Text(
                              suggestion
                                  .split('-')
                                  .lastWhere((w) => w.isNotEmpty),
                              style: const TextStyle(fontSize: 16),
                            ),
                          ),
                        ),
                      ),
                  ],
                ),
                const SizedBox(height: 14),
              ],
              const Text(
                'Words come from the wormhole word list. Tap one to complete it.',
                style: subStyle,
              ),
            ],
          ),
          foot: [
            Primary('Connect', onPressed: valid ? _connect : null),
            Secondary(
              'Scan instead',
              icon: Icons.qr_code_scanner,
              onPressed: _scan,
            ),
          ],
        );
      },
    );
  }
}
