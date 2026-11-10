import 'package:flutter/material.dart';

import '../api/stores.dart';
import '../models.dart';
import '../theme.dart';
import '../widgets/timer_card.dart';

/// Display for a client machine: shows the timers assigned to it by the server,
/// as reported by the local firefight-agent.
class ClientPage extends StatefulWidget {
  const ClientPage({super.key, required this.agentPort, required this.onSettings, this.onFiring});
  final int agentPort;
  final VoidCallback onSettings;

  /// Called when an assigned timer starts firing, e.g. to bring a tray-hidden window forward.
  final VoidCallback? onFiring;

  @override
  State<ClientPage> createState() => _ClientPageState();
}

class _ClientPageState extends State<ClientPage> {
  late final store = AgentStore(port: widget.agentPort)
    ..addListener(_checkFiring)
    ..connect();

  /// Runs seen firing, so each run alerts once.
  final _firing = <String>{};

  void _checkFiring() {
    final now = {
      for (final r in store.runs.values)
        if (r.state == RunState.firing) r.runId,
    };
    if (now.difference(_firing).isNotEmpty) widget.onFiring?.call();
    _firing
      ..clear()
      ..addAll(now);
  }

  @override
  void dispose() {
    store.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: store,
      builder: (context, _) => Scaffold(
        body: Column(
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(28, 18, 20, 18),
              child: Row(
                children: [
                  const Icon(Icons.timer_outlined, color: Palette.amber),
                  const SizedBox(width: 10),
                  Text(
                    store.name.isEmpty ? 'Firefight' : store.name,
                    style: const TextStyle(fontSize: 19, fontWeight: FontWeight.w600),
                  ),
                  const SizedBox(width: 16),
                  _StatusBadge(store: store),
                  const Spacer(),
                  IconButton(
                    onPressed: widget.onSettings,
                    icon: const Icon(Icons.settings_outlined),
                    tooltip: 'Settings',
                  ),
                ],
              ),
            ),
            const Divider(height: 1),
            Expanded(child: _body()),
          ],
        ),
      ),
    );
  }

  Widget _body() {
    if (!store.connected) {
      return _Message(
        icon: Icons.link_off,
        title: 'firefight-agent is not running',
        detail: 'Start the agent on this machine (it listens on 127.0.0.1:${widget.agentPort}).',
      );
    }
    if (store.status == AgentStatus.pending) {
      return _Message(
        icon: Icons.hourglass_top,
        title: 'Waiting for approval',
        detail: 'Approve this machine in the control deck.\nClient id ${store.clientId?.substring(0, 8) ?? '…'}',
      );
    }
    if (store.timers.isEmpty) {
      return const _Message(
        icon: Icons.inbox_outlined,
        title: 'No timers assigned',
        detail: 'Drag a timer onto this client in the control deck.',
      );
    }
    return LayoutBuilder(
      builder: (context, box) {
        // Fewer, bigger cards than the deck: this is meant to be read from a distance.
        final n = store.timers.length;
        final columns = n == 1 ? 1 : (box.maxWidth / 520).floor().clamp(1, n);
        const gap = 24.0;
        final width = ((box.maxWidth - 56 - gap * (columns - 1)) / columns).clamp(300.0, 720.0);
        return SingleChildScrollView(
          padding: const EdgeInsets.all(28),
          child: Center(
            child: Wrap(
              spacing: gap,
              runSpacing: gap,
              alignment: WrapAlignment.center,
              children: [
                for (final def in store.timers)
                  SizedBox(
                    width: width,
                    child: TimerCard(
                      def: def,
                      run: store.runs[def.id],
                      serverNow: store.clock.now,
                      ringSize: (width * 0.4).clamp(96.0, 240.0),
                      footer: def.allowClientDismiss && store.runs[def.id]?.state == RunState.firing
                          ? FilledButton(onPressed: () => store.dismiss(def.id), child: const Text('Dismiss'))
                          : null,
                    ),
                  ),
              ],
            ),
          ),
        );
      },
    );
  }
}

class _StatusBadge extends StatelessWidget {
  const _StatusBadge({required this.store});
  final AgentStore store;

  @override
  Widget build(BuildContext context) {
    final (color, text) = !store.connected
        ? (Palette.red, 'agent not running')
        : switch (store.status) {
            AgentStatus.approved => (Palette.green, 'connected'),
            AgentStatus.pending => (Palette.amber, 'pending approval'),
            AgentStatus.disconnected => (Palette.red, 'server unreachable'),
          };
    return Row(
      children: [
        Icon(Icons.circle, size: 9, color: color),
        const SizedBox(width: 6),
        Text(text, style: const TextStyle(color: Palette.muted, fontSize: 12)),
      ],
    );
  }
}

class _Message extends StatelessWidget {
  const _Message({required this.icon, required this.title, required this.detail});
  final IconData icon;
  final String title;
  final String detail;

  @override
  Widget build(BuildContext context) => Center(
    child: Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        Icon(icon, size: 40, color: Palette.muted),
        const SizedBox(height: 14),
        Text(title, style: const TextStyle(fontSize: 18, fontWeight: FontWeight.w600)),
        const SizedBox(height: 6),
        Text(
          detail,
          textAlign: TextAlign.center,
          style: const TextStyle(color: Palette.muted),
        ),
      ],
    ),
  );
}
