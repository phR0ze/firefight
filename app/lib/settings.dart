import 'dart:convert';
import 'dart:io';

import 'package:path_provider/path_provider.dart';

enum AppMode { server, client }

/// Persisted app settings: `$XDG_CONFIG_HOME/firefight/app.json` on Linux, the
/// app-private support directory on Android. Only connection details live here;
/// timers and clients are always read from the server.
class AppSettings {
  AppSettings({this.mode, this.server = 'localhost:7878', this.adminToken, this.agentPort = 7879});

  AppMode? mode;
  String server;
  String? adminToken;
  int agentPort;

  /// Android runs only the control deck: the client display needs a local
  /// firefight-agent, which is Linux only.
  static bool get deckOnly => Platform.isAndroid;

  static Future<File> _file() async => Platform.isAndroid
      ? File('${(await getApplicationSupportDirectory()).path}/app.json')
      : File('${_xdg('XDG_CONFIG_HOME', '.config')}/firefight/app.json');

  static String _xdg(String env, String fallback) =>
      Platform.environment[env] ?? '${Platform.environment['HOME'] ?? '.'}/$fallback';

  /// Admin token written by a server running on this machine, if any.
  static String? localAdminToken() {
    if (Platform.isAndroid) return null;
    final f = File('${_xdg('XDG_DATA_HOME', '.local/share')}/firefight/admin_token');
    try {
      final t = f.readAsStringSync().trim();
      return t.isEmpty ? null : t;
    } on FileSystemException {
      return null;
    }
  }

  static Future<AppSettings> load() async {
    AppSettings s;
    try {
      final j = jsonDecode(await (await _file()).readAsString()) as Map<String, dynamic>;
      s = AppSettings(
        mode: j['mode'] == null ? null : AppMode.values.byName(j['mode'] as String),
        server: j['server'] as String? ?? 'localhost:7878',
        adminToken: j['admin_token'] as String?,
        agentPort: j['agent_port'] as int? ?? 7879,
      );
    } on Object {
      // A phone never runs the server itself, so don't suggest localhost there.
      s = deckOnly ? AppSettings(server: '') : AppSettings();
    }
    if (deckOnly) s.mode = AppMode.server;
    return s;
  }

  Future<void> save() async {
    final f = await _file();
    await f.parent.create(recursive: true);
    await f.writeAsString(
      const JsonEncoder.withIndent('  ')
          .convert({'mode': mode?.name, 'server': server, 'admin_token': adminToken, 'agent_port': agentPort}),
    );
    // The file may hold the admin token. Android's app directory is already private.
    if (!Platform.isAndroid) await Process.run('chmod', ['600', f.path]);
  }

  /// Apply `--mode=`, `--server=`, `--token=` and `--agent-port=` overrides.
  void applyArgs(List<String> args) {
    for (final a in args) {
      final i = a.indexOf('=');
      if (!a.startsWith('--') || i < 0) continue;
      final (k, v) = (a.substring(2, i), a.substring(i + 1));
      switch (k) {
        case 'mode':
          mode = AppMode.values.asNameMap()[v] ?? mode;
        case 'server':
          server = v;
        case 'token':
          adminToken = v;
        case 'agent-port':
          agentPort = int.tryParse(v) ?? agentPort;
      }
    }
  }
}
