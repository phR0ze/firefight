import 'dart:io' show Platform;

import 'package:flutter/material.dart';

import '../api/admin_api.dart';
import '../api/stores.dart';
import '../models.dart';
import '../theme.dart';
import '../widgets/timer_card.dart';
import 'client_card.dart';
import 'timer_editor.dart';

/// Run an API call, surfacing failures as a snackbar.
Future<void> guard(BuildContext context, Future<void> Function() call) async {
  try {
    await call();
  } on Object catch (e) {
    if (context.mounted) {
      ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text('$e'), backgroundColor: Palette.red));
    }
  }
}

class DeckPage extends StatefulWidget {
  const DeckPage({super.key, required this.server, required this.token, required this.onSettings});
  final String server;
  final String token;
  final VoidCallback onSettings;

  @override
  State<DeckPage> createState() => _DeckPageState();
}

class _DeckPageState extends State<DeckPage> {
  late final store = DeckStore(server: widget.server, token: widget.token)..connect();
  late final api = AdminApi(server: widget.server, token: widget.token);

  /// Selected tab in the compact (phone) layout: 0 timers, 1 clients.
  int _tab = 0;

  @override
  void dispose() {
    store.dispose();
    super.dispose();
  }

  Future<void> _edit([TimerDef? def]) async {
    final saved = await showDialog<TimerDef>(
      context: context,
      builder: (_) => TimerEditor(
        initial: def,
        clients: store.clients.values.where((c) => c.online && c.status == ClientStatus.approved).toList(),
        onTestRun: (clientId, actions) => api.testRun(clientId, actions),
      ),
    );
    if (saved == null || !mounted) return;
    await guard(context, () => def == null ? api.createTimer(saved) : api.updateTimer(saved));
  }

  Future<void> _delete(TimerDef def) async {
    final ok = await showDialog<bool>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: Text('Delete "${def.name}"?'),
        content: const Text('It will be unassigned from every client.'),
        actions: [
          TextButton(onPressed: () => Navigator.pop(ctx, false), child: const Text('Cancel')),
          FilledButton(
            style: FilledButton.styleFrom(backgroundColor: Palette.red),
            onPressed: () => Navigator.pop(ctx, true),
            child: const Text('Delete'),
          ),
        ],
      ),
    );
    if (ok == true && mounted) await guard(context, () => api.deleteTimer(def.id));
  }

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: store,
      builder: (context, _) => LayoutBuilder(
        builder: (context, box) {
          final compact = box.maxWidth < compactBreakpoint;
          final grid = _TimerGrid(store: store, api: api, onEdit: _edit, onDelete: _delete, compact: compact);
          final clients = ClientsPanel(store: store, api: api);
          final Widget body;
          if (compact) {
            body = _tab == 0 ? grid : clients;
          } else if (box.maxWidth >= 1100) {
            body = Row(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Expanded(child: grid),
                const VerticalDivider(width: 1),
                SizedBox(width: 340, child: clients),
              ],
            );
          } else {
            body = Column(
              children: [
                Expanded(flex: 3, child: grid),
                const Divider(height: 1),
                Expanded(flex: 2, child: clients),
              ],
            );
          }
          final pending = store.clients.values.where((c) => c.status == ClientStatus.pending).length;
          return Scaffold(
            body: SafeArea(
              child: Column(
                children: [
                  _Header(
                    compact: compact,
                    connected: store.connected,
                    error: store.error,
                    onNew: store.connected ? () => _edit() : null,
                    onBulk: (op) =>
                        store.connected && _anyApplies(store, op) ? () => guard(context, () => api.bulkAll(op)) : null,
                    onSettings: widget.onSettings,
                  ),
                  const Divider(height: 1),
                  Expanded(child: body),
                ],
              ),
            ),
            floatingActionButton: compact && _tab == 0 && store.connected
                ? FloatingActionButton(onPressed: () => _edit(), tooltip: 'New timer', child: const Icon(Icons.add))
                : null,
            bottomNavigationBar: compact
                ? NavigationBar(
                    selectedIndex: _tab,
                    onDestinationSelected: (i) => setState(() => _tab = i),
                    destinations: [
                      const NavigationDestination(icon: Icon(Icons.timer_outlined), label: 'Timers'),
                      NavigationDestination(
                        icon: Badge(
                          isLabelVisible: pending > 0,
                          label: Text('$pending'),
                          child: const Icon(Icons.devices_outlined),
                        ),
                        label: 'Clients',
                      ),
                    ],
                  )
                : null,
          );
        },
      ),
    );
  }
}

/// Whether [op] would change any timer assigned to an approved client.
bool _anyApplies(DeckStore store, BulkOp op) => op.appliesToAny(
  store.clients.values.where((c) => c.status == ClientStatus.approved).expand((c) => c.assignedTimers),
  store.timers,
  store.runs,
);

class _Header extends StatelessWidget {
  const _Header({
    required this.compact,
    required this.connected,
    required this.error,
    required this.onNew,
    required this.onBulk,
    required this.onSettings,
  });

  /// Phone layout: bulk operations and settings move into an overflow menu.
  final bool compact;
  final bool connected;
  final String? error;
  final VoidCallback? onNew;

  /// Handler for a bulk button, or null to disable it.
  final VoidCallback? Function(BulkOp) onBulk;
  final VoidCallback onSettings;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: compact ? const EdgeInsets.fromLTRB(16, 8, 4, 8) : const EdgeInsets.fromLTRB(28, 18, 28, 18),
      child: Row(
        children: [
          const Icon(Icons.timer_outlined, color: Palette.amber),
          const SizedBox(width: 10),
          const Text('Firefight', style: TextStyle(fontSize: 19, fontWeight: FontWeight.w600)),
          const SizedBox(width: 16),
          Tooltip(
            message: connected ? 'connected' : (error ?? 'connecting…'),
            child: Row(
              children: [
                Icon(Icons.circle, size: 9, color: connected ? Palette.green : Palette.red),
                const SizedBox(width: 6),
                if (!connected && !compact)
                  Text(error ?? 'connecting…', style: const TextStyle(color: Palette.muted, fontSize: 12)),
              ],
            ),
          ),
          const Spacer(),
          if (compact)
            PopupMenuButton<VoidCallback>(
              tooltip: 'More',
              icon: const Icon(Icons.more_vert),
              onSelected: (action) => action(),
              itemBuilder: (_) => [
                for (final op in BulkOp.values)
                  PopupMenuItem(
                    value: onBulk(op),
                    enabled: onBulk(op) != null,
                    child: ListTile(leading: Icon(op.icon), title: Text('${op.label} all'), dense: true),
                  ),
                const PopupMenuDivider(),
                PopupMenuItem(
                  value: onSettings,
                  child: const ListTile(
                    leading: Icon(Icons.settings_outlined),
                    title: Text('Connection settings'),
                    dense: true,
                  ),
                ),
              ],
            )
          else ...[
            IconButton(
              onPressed: onSettings,
              icon: const Icon(Icons.settings_outlined),
              tooltip: 'Connection settings',
            ),
            const SizedBox(width: 8),
            for (final op in BulkOp.values) ...[
              Tooltip(
                message: '${op.label} every timer assigned to a client',
                child: OutlinedButton.icon(
                  onPressed: onBulk(op),
                  icon: Icon(op.icon, size: 20),
                  label: Text('${op.label} all'),
                ),
              ),
              const SizedBox(width: 8),
            ],
            FilledButton(onPressed: onNew, child: const Text('+ New timer')),
          ],
        ],
      ),
    );
  }
}

class _TimerGrid extends StatelessWidget {
  const _TimerGrid({
    required this.store,
    required this.api,
    required this.onEdit,
    required this.onDelete,
    required this.compact,
  });
  final DeckStore store;
  final AdminApi api;
  final void Function(TimerDef) onEdit;
  final void Function(TimerDef) onDelete;

  /// Phone layout: tighter padding, and no dragging since clients are on another tab.
  final bool compact;

  @override
  Widget build(BuildContext context) {
    if (store.timers.isEmpty) {
      return Center(
        child: Text(
          store.connected
              ? 'No timers yet. Create one with ${compact ? 'the + button' : '"+ New timer"'}.'
              : 'Waiting for the server…',
          style: const TextStyle(color: Palette.muted),
        ),
      );
    }
    return SingleChildScrollView(
      padding: EdgeInsets.all(compact ? 16 : 28),
      child: LayoutBuilder(
        builder: (context, box) {
          const gap = 20.0;
          final columns = (box.maxWidth / 360).floor().clamp(1, 6);
          final width = (box.maxWidth - gap * (columns - 1)) / columns;
          return Wrap(
            spacing: gap,
            runSpacing: gap,
            children: [
              for (final def in store.timers.values)
                SizedBox(
                  width: width,
                  child: _DeckTimerCard(
                    def: def,
                    run: store.runs[def.id],
                    store: store,
                    api: api,
                    onEdit: () => onEdit(def),
                    onDelete: () => onDelete(def),
                    draggable: !compact,
                  ),
                ),
            ],
          );
        },
      ),
    );
  }
}

class _DeckTimerCard extends StatelessWidget {
  const _DeckTimerCard({
    required this.def,
    required this.run,
    required this.store,
    required this.api,
    required this.onEdit,
    required this.onDelete,
    required this.draggable,
  });
  final TimerDef def;
  final TimerRun? run;
  final DeckStore store;
  final AdminApi api;
  final VoidCallback onEdit;
  final VoidCallback onDelete;

  /// Whether the card can be dragged onto a client card.
  final bool draggable;

  @override
  Widget build(BuildContext context) {
    final state = run?.state ?? RunState.idle;
    final isAtTime = def.kind is AtTime;
    void op(String name, {int? secs}) => guard(context, () => api.control(def.id, name, addSecs: secs));

    final (label, action, primary) = switch (state) {
      RunState.firing => ('Dismiss', () => op('dismiss'), true),
      RunState.running when isAtTime => ('Disarm', () => op('reset'), false),
      RunState.running => ('Pause', () => op('pause'), false),
      RunState.paused => ('Resume', () => op('start'), false),
      RunState.idle => (isAtTime ? 'Arm' : 'Start', () => op('start'), false),
    };

    final card = TimerCard(
      def: def,
      run: run,
      serverNow: store.clock.now,
      trailing: PopupMenuButton<String>(
        tooltip: 'More',
        padding: EdgeInsets.zero,
        icon: const Icon(Icons.more_vert, size: 18, color: Palette.muted),
        onSelected: (v) => v == 'edit' ? onEdit() : onDelete(),
        itemBuilder: (_) => const [
          PopupMenuItem(value: 'edit', child: Text('Edit')),
          PopupMenuItem(value: 'delete', child: Text('Delete')),
        ],
      ),
      footer: Row(
        children: [
          Expanded(
            child: FilledButton(style: primary ? null : neutralButton(), onPressed: action, child: Text(label)),
          ),
          const SizedBox(width: 8),
          Tooltip(
            message: state == RunState.firing ? 'Snooze 1 minute' : 'Add 1 minute',
            child: OutlinedButton(
              onPressed: isAtTime && state != RunState.firing ? null : () => op('add', secs: 60),
              child: const Text('+1m'),
            ),
          ),
          const SizedBox(width: 8),
          Tooltip(
            message: 'Reset',
            child: OutlinedButton(
              onPressed: () => op('reset'),
              style: OutlinedButton.styleFrom(padding: EdgeInsets.zero),
              child: const Icon(Icons.replay, size: 18),
            ),
          ),
        ],
      ),
    );

    if (!draggable) return card;

    // Drag a card onto a client to assign it.
    final feedback = Material(
      color: Colors.transparent,
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 10),
        decoration: BoxDecoration(
          color: Palette.card,
          borderRadius: BorderRadius.circular(10),
          border: Border.all(color: Palette.amber),
          boxShadow: const [BoxShadow(blurRadius: 16, color: Colors.black54)],
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            const Icon(Icons.timer_outlined, size: 16, color: Palette.amber),
            const SizedBox(width: 8),
            Text(def.name, style: const TextStyle(color: Palette.text, fontSize: 14)),
          ],
        ),
      ),
    );
    // On touch screens a plain Draggable would capture scroll gestures, so drag
    // only after a long press there.
    if (Platform.isAndroid) {
      return LongPressDraggable<String>(
        data: def.id,
        feedback: feedback,
        childWhenDragging: Opacity(opacity: 0.5, child: card),
        child: card,
      );
    }
    return Draggable<String>(
      data: def.id,
      feedback: feedback,
      childWhenDragging: Opacity(opacity: 0.5, child: card),
      child: MouseRegion(cursor: SystemMouseCursors.grab, child: card),
    );
  }
}
