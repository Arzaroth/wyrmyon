import 'package:shared_preferences/shared_preferences.dart';

import 'engine.dart';

abstract class SettingsStore {
  Future<Settings?> load();
  Future<void> save(Settings? settings);
}

class PreferencesStore implements SettingsStore {
  static const _keys = ('mailbox', 'transit_helper', 'iroh_relays');

  @override
  Future<Settings?> load() async {
    final prefs = await SharedPreferences.getInstance();
    final mailbox = prefs.getString(_keys.$1);
    final helper = prefs.getString(_keys.$2);
    final relays = prefs.getString(_keys.$3);
    if (mailbox == null || helper == null || relays == null) return null;
    return Settings(
      mailbox: mailbox,
      transitHelper: helper,
      irohRelays: relays,
    );
  }

  @override
  Future<void> save(Settings? settings) async {
    final prefs = await SharedPreferences.getInstance();
    if (settings == null) {
      await Future.wait([
        for (final key in [_keys.$1, _keys.$2, _keys.$3]) prefs.remove(key),
      ]);
      return;
    }
    await prefs.setString(_keys.$1, settings.mailbox);
    await prefs.setString(_keys.$2, settings.transitHelper);
    await prefs.setString(_keys.$3, settings.irohRelays);
  }
}
