import 'package:flutter/material.dart';

import '../app.dart';

class TypeCodePage extends StatefulWidget {
  const TypeCodePage({super.key});

  static Route<String> route() =>
      MaterialPageRoute(builder: (_) => const TypeCodePage());

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

  @override
  Widget build(BuildContext context) {
    final engine = Services.of(context).engine;
    return Scaffold(
      appBar: AppBar(title: const Text('Enter code')),
      body: ListenableBuilder(
        listenable: _typed,
        builder: (context, _) {
          final suggestions = engine.completions(_typed.text);
          final valid = engine.scan(_typed.text) != null;
          return ListView(
            padding: const EdgeInsets.all(16),
            children: [
              TextField(
                controller: _typed,
                autofocus: true,
                autocorrect: false,
                enableSuggestions: false,
                keyboardType: TextInputType.visiblePassword,
                decoration: const InputDecoration(
                  border: OutlineInputBorder(),
                  hintText: '7-guitarist-revenge',
                ),
                onSubmitted: (_) => _connect(),
              ),
              const SizedBox(height: 12),
              Wrap(
                spacing: 8,
                runSpacing: 8,
                children: [
                  for (final suggestion in suggestions.take(12))
                    ActionChip(
                      label: Text(
                        suggestion.split('-').lastWhere((w) => w.isNotEmpty),
                      ),
                      onPressed: () => _complete(suggestion),
                    ),
                ],
              ),
              const SizedBox(height: 16),
              FilledButton(
                onPressed: valid ? _connect : null,
                child: const Text('Connect'),
              ),
            ],
          );
        },
      ),
    );
  }
}
