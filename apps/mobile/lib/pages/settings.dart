import 'package:flutter/material.dart';

import '../app.dart';
import '../engine.dart';
import '../ui.dart';

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
    Widget field(TextEditingController controller, String label, String help) =>
        Padding(
          padding: const EdgeInsets.only(bottom: 22),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            spacing: 8,
            children: [
              Label(label),
              TextField(
                controller: controller,
                autocorrect: false,
                enableSuggestions: false,
                keyboardType: TextInputType.url,
                cursorColor: Palette.cyan,
                style: const TextStyle(fontFamily: mono, fontSize: 16),
                decoration: InputDecoration(
                  semanticCounterText: label,
                  helperText: help,
                  helperMaxLines: 3,
                ),
              ),
            ],
          ),
        );
    return Screen(
      title: 'Settings',
      body: ListView(
        padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 8),
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
            Text(problem, style: const TextStyle(color: Palette.error)),
        ],
      ),
      foot: [
        Primary('Save', onPressed: _save),
        Center(
          child: Ghost(
            'Restore the defaults',
            onPressed: () => setState(() {
              _problem = null;
              _fill(Services.of(context).engine.defaults);
            }),
          ),
        ),
      ],
    );
  }
}
