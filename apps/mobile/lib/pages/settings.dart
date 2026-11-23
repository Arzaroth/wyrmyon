import 'package:flutter/material.dart';

import '../app.dart';
import '../engine.dart';

class SettingsPage extends StatefulWidget {
  const SettingsPage({super.key});

  static Route<void> route() =>
      MaterialPageRoute(builder: (_) => const SettingsPage());

  @override
  State<SettingsPage> createState() => _SettingsPageState();
}

class _SettingsPageState extends State<SettingsPage> {
  final _mailbox = TextEditingController();
  final _helper = TextEditingController();
  final _relays = TextEditingController();
  String? _problem;
  bool _loaded = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_loaded) return;
    _loaded = true;
    _fill(Services.of(context).engine.settings);
  }

  @override
  void dispose() {
    _mailbox.dispose();
    _helper.dispose();
    _relays.dispose();
    super.dispose();
  }

  void _fill(Settings settings) {
    _mailbox.text = settings.mailbox;
    _helper.text = settings.transitHelper;
    _relays.text = settings.irohRelays;
  }

  Future<void> _save() async {
    final services = Services.of(context);
    final navigator = Navigator.of(context);
    final settings = Settings(
      mailbox: _mailbox.text.trim(),
      transitHelper: _helper.text.trim(),
      irohRelays: _relays.text.trim(),
    );
    final problem = services.engine.check(settings);
    if (problem != null) {
      setState(() => _problem = problem);
      return;
    }
    services.engine.settings = settings;
    final defaults = services.engine.defaults;
    await services.settings.save(settings == defaults ? null : settings);
    navigator.pop();
  }

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    Widget field(TextEditingController controller, String label, String help) =>
        Padding(
          padding: const EdgeInsets.only(bottom: 16),
          child: TextField(
            controller: controller,
            autocorrect: false,
            enableSuggestions: false,
            keyboardType: TextInputType.url,
            decoration: InputDecoration(
              border: const OutlineInputBorder(),
              labelText: label,
              helperText: help,
              helperMaxLines: 3,
            ),
          ),
        );
    return Scaffold(
      appBar: AppBar(title: const Text('Settings')),
      body: ListView(
        padding: const EdgeInsets.all(16),
        children: [
          field(
            _mailbox,
            'Mailbox server',
            'Where the two sides meet. Both must use the same one.',
          ),
          field(
            _helper,
            'Transit relay',
            'Carries the data when the two sides cannot reach each other '
                'directly, for classic transfers.',
          ),
          field(
            _relays,
            'iroh relays',
            'default, disabled, or a comma-separated list of relay URLs.',
          ),
          if (_problem case final problem?)
            Padding(
              padding: const EdgeInsets.only(bottom: 16),
              child: Text(problem, style: TextStyle(color: scheme.error)),
            ),
          FilledButton(onPressed: _save, child: const Text('Save')),
          const SizedBox(height: 8),
          TextButton(
            onPressed: () => setState(() {
              _problem = null;
              _fill(Services.of(context).engine.defaults);
            }),
            child: const Text('Restore the defaults'),
          ),
        ],
      ),
    );
  }
}
