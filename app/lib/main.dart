import 'dart:io' show Platform;

import 'package:flutter/material.dart';

import 'client/client_page.dart';
import 'client/tray.dart';
import 'deck/deck_page.dart';
import 'settings.dart';
import 'theme.dart';

/// Arguments: `--mode=server|client`, `--server=<host:port>`, `--token=<admin token>`,
/// `--token-file=<path>`, `--agent-port=<port>`. Values given here are remembered for the next
/// launch. The Linux client display is run by firefight-agent and cannot be quit: it lives in
/// the system tray (`--start-hidden` starts it there), or with `--no-tray` closing the window
/// minimizes it. Its mode and agent port come from the agent, so it has no settings screen.
Future<void> main(List<String> args) async {
  WidgetsFlutterBinding.ensureInitialized();
  final settings = (await AppSettings.load())..applyArgs(args);
  settings.adminToken ??= AppSettings.localAdminToken();
  if (Platform.isLinux && settings.mode == AppMode.client) {
    final tray = args.contains('--no-tray') ? null : ClientTray.start();
    if (tray == null) ClientTray.keepOpen();
    runApp(FirefightApp(settings: settings, tray: tray));
  } else {
    runApp(FirefightApp(settings: settings));
  }
}

class FirefightApp extends StatefulWidget {
  const FirefightApp({super.key, required this.settings, this.tray});
  final AppSettings settings;

  /// Present when the client display runs in the system tray.
  final ClientTray? tray;

  @override
  State<FirefightApp> createState() => _FirefightAppState();
}

class _FirefightAppState extends State<FirefightApp> {
  late AppSettings settings = widget.settings;
  late bool editing = _needsSetup;
  // Changing it rebuilds pages so they reconnect with new settings.
  int generation = 0;

  bool get _needsSetup => settings.mode == null || (settings.mode == AppMode.server && settings.adminToken == null);

  void _openSettings() => setState(() => editing = true);

  Future<void> _saved(AppSettings s) async {
    await s.save();
    setState(() {
      settings = s;
      editing = false;
      generation++;
    });
  }

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Firefight',
      debugShowCheckedModeBanner: false,
      theme: buildTheme(),
      home: editing
          ? SetupPage(
              initial: settings,
              deckOnly: AppSettings.deckOnly,
              onSave: _saved,
              onCancel: _needsSetup ? null : () => setState(() => editing = false),
            )
          : switch (settings.mode!) {
              AppMode.server => DeckPage(
                key: ValueKey(generation),
                server: settings.server,
                token: settings.adminToken!,
                onSettings: _openSettings,
              ),
              AppMode.client => ClientPage(
                key: ValueKey(generation),
                agentPort: settings.agentPort,
                onFiring: widget.tray?.show,
              ),
            },
    );
  }
}

/// First-run / settings screen: pick the mode and how to connect.
class SetupPage extends StatefulWidget {
  const SetupPage({super.key, required this.initial, required this.onSave, this.onCancel, this.deckOnly = false});
  final AppSettings initial;

  /// Hide the mode choice and only offer the control deck (Android).
  final bool deckOnly;
  final ValueChanged<AppSettings> onSave;
  final VoidCallback? onCancel;

  @override
  State<SetupPage> createState() => _SetupPageState();
}

class _SetupPageState extends State<SetupPage> {
  late AppMode mode = widget.deckOnly ? AppMode.server : (widget.initial.mode ?? AppMode.server);
  late final server = TextEditingController(text: widget.initial.server);
  late final token = TextEditingController(text: widget.initial.adminToken ?? '');
  late final port = TextEditingController(text: '${widget.initial.agentPort}');
  String? error;

  void _save() {
    final p = int.tryParse(port.text);
    if (mode == AppMode.server && (server.text.trim().isEmpty || token.text.trim().isEmpty)) {
      setState(() => error = 'Server address and admin token are required.');
      return;
    }
    if (mode == AppMode.client && (p == null || p <= 0 || p > 65535)) {
      setState(() => error = 'Enter a valid agent port.');
      return;
    }
    widget.onSave(
      AppSettings(
        mode: mode,
        server: server.text.trim(),
        adminToken: token.text.trim().isEmpty ? null : token.text.trim(),
        agentPort: p ?? widget.initial.agentPort,
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: SafeArea(
        child: Center(
          child: ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 460),
            child: ListView(
              shrinkWrap: true,
              padding: const EdgeInsets.all(32),
              children: [
                const Row(
                  children: [
                    Icon(Icons.timer_outlined, color: Palette.amber),
                    SizedBox(width: 10),
                    Text('Firefight', style: TextStyle(fontSize: 22, fontWeight: FontWeight.w600)),
                  ],
                ),
                const SizedBox(height: 24),
                if (!widget.deckOnly) ...[
                  SegmentedButton<AppMode>(
                    segments: const [
                      ButtonSegment(
                        value: AppMode.server,
                        label: Text('Control deck'),
                        icon: Icon(Icons.dashboard_outlined),
                      ),
                      ButtonSegment(
                        value: AppMode.client,
                        label: Text('Client display'),
                        icon: Icon(Icons.tv_outlined),
                      ),
                    ],
                    selected: {mode},
                    onSelectionChanged: (s) => setState(() => mode = s.first),
                  ),
                  const SizedBox(height: 24),
                ],
                if (mode == AppMode.server) ...[
                  TextField(
                    controller: server,
                    keyboardType: TextInputType.url,
                    autocorrect: false,
                    decoration: const InputDecoration(labelText: 'Server (host:port)', hintText: 'server-host:7878'),
                  ),
                  const SizedBox(height: 14),
                  TextField(
                    controller: token,
                    obscureText: true,
                    decoration: const InputDecoration(
                      labelText: 'Admin token',
                      helperText: 'Found in the server\'s data directory (admin_token).',
                    ),
                  ),
                ] else
                  TextField(
                    controller: port,
                    decoration: const InputDecoration(
                      labelText: 'Local agent port',
                      helperText: 'The firefight-agent on this machine (--ui-port).',
                    ),
                  ),
                if (error != null) ...[
                  const SizedBox(height: 14),
                  Text(error!, style: const TextStyle(color: Palette.red)),
                ],
                const SizedBox(height: 24),
                Row(
                  mainAxisAlignment: MainAxisAlignment.end,
                  children: [
                    if (widget.onCancel != null) ...[
                      OutlinedButton(onPressed: widget.onCancel, child: const Text('Cancel')),
                      const SizedBox(width: 10),
                    ],
                    FilledButton(onPressed: _save, child: const Text('Continue')),
                  ],
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
