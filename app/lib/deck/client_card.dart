import 'package:flutter/material.dart';

import '../api/admin_api.dart';
import '../api/stores.dart';
import '../models.dart';
import '../theme.dart';
import '../widgets/timer_card.dart';
import 'deck_page.dart';

extension BulkOpIcon on BulkOp {
  IconData get icon => switch (this) {
    BulkOp.start => Icons.play_arrow_rounded,
    BulkOp.pause => Icons.pause_rounded,
    BulkOp.reset => Icons.restart_alt_rounded,
  };
}

/// Side panel listing clients: pending ones to approve, approved ones as drop targets.
class ClientsPanel extends StatelessWidget {
  const ClientsPanel({super.key, required this.store, required this.api});
  final DeckStore store;
  final AdminApi api;

  @override
  Widget build(BuildContext context) {
    final pending = store.clients.values.where((c) => c.status == ClientStatus.pending).toList();
    final approved = store.clients.values.where((c) => c.status == ClientStatus.approved).toList()
      ..sort((a, b) => a.name.toLowerCase().compareTo(b.name.toLowerCase()));
    return ColoredBox(
      color: Palette.panel,
      child: ListView(
        padding: const EdgeInsets.all(20),
        children: [
          const _SectionTitle('CLIENTS', hint: 'Drag a timer onto a client to assign it'),
          if (approved.isEmpty)
            const Padding(
              padding: EdgeInsets.symmetric(vertical: 12),
              child: Text(
                'No clients yet. Start firefight-agent on a machine and approve it here.',
                style: TextStyle(color: Palette.muted, fontSize: 13),
              ),
            ),
          for (final c in approved) ClientCard(client: c, store: store, api: api),
          if (pending.isNotEmpty) ...[
            const SizedBox(height: 12),
            const _SectionTitle('AWAITING APPROVAL', hint: 'Approve only machines you recognise'),
            for (final c in pending) _PendingCard(client: c, api: api),
          ],
        ],
      ),
    );
  }
}

class _SectionTitle extends StatelessWidget {
  const _SectionTitle(this.text, {required this.hint});
  final String text;
  final String hint;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.only(bottom: 12),
    child: Tooltip(
      message: hint,
      child: Text(text, style: const TextStyle(color: Palette.muted, fontSize: 11, letterSpacing: 1)),
    ),
  );
}

class _OnlineDot extends StatelessWidget {
  const _OnlineDot(this.online);
  final bool online;

  @override
  Widget build(BuildContext context) => Tooltip(
    message: online ? 'online' : 'offline',
    child: Icon(Icons.circle, size: 9, color: online ? Palette.green : Palette.slate),
  );
}

class _PendingCard extends StatelessWidget {
  const _PendingCard({required this.client, required this.api});
  final ClientView client;
  final AdminApi api;

  @override
  Widget build(BuildContext context) {
    return Container(
      margin: const EdgeInsets.only(bottom: 12),
      padding: const EdgeInsets.all(14),
      decoration: BoxDecoration(
        color: Palette.card,
        borderRadius: BorderRadius.circular(12),
        border: Border.all(color: Palette.border),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              _OnlineDot(client.online),
              const SizedBox(width: 8),
              Expanded(
                child: Text(client.name, style: const TextStyle(fontWeight: FontWeight.w600)),
              ),
            ],
          ),
          const SizedBox(height: 4),
          Text(
            '${client.hostname} · ${client.id.substring(0, 8)}',
            style: const TextStyle(color: Palette.muted, fontSize: 12),
          ),
          const SizedBox(height: 10),
          Row(
            children: [
              Expanded(
                child: FilledButton(
                  onPressed: () => guard(context, () => api.approveClient(client.id)),
                  child: const Text('Approve'),
                ),
              ),
              const SizedBox(width: 8),
              OutlinedButton(
                onPressed: () => guard(context, () => api.removeClient(client.id)),
                child: const Text('Reject'),
              ),
            ],
          ),
        ],
      ),
    );
  }
}

class ClientCard extends StatelessWidget {
  const ClientCard({super.key, required this.client, required this.store, required this.api});
  final ClientView client;
  final DeckStore store;
  final AdminApi api;

  Future<void> _rename(BuildContext context) async {
    final ctrl = TextEditingController(text: client.name);
    final name = await showDialog<String>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: const Text('Rename client'),
        content: TextField(controller: ctrl, autofocus: true, onSubmitted: (v) => Navigator.pop(ctx, v)),
        actions: [
          TextButton(onPressed: () => Navigator.pop(ctx), child: const Text('Cancel')),
          FilledButton(onPressed: () => Navigator.pop(ctx, ctrl.text), child: const Text('Save')),
        ],
      ),
    );
    if (name != null && name.trim().isNotEmpty && context.mounted) {
      await guard(context, () => api.renameClient(client.id, name.trim()));
    }
  }

  /// Checklist of every timer, for assigning without drag-and-drop (phones).
  Future<void> _assign(BuildContext context) async {
    final ids = await showModalBottomSheet<List<String>>(
      context: context,
      isScrollControlled: true,
      showDragHandle: true,
      builder: (_) => AssignSheet(client: client, timers: store.timers.values.toList()),
    );
    if (ids != null && context.mounted) await guard(context, () => api.setAssignments(client.id, ids));
  }

  Future<void> _remove(BuildContext context) async {
    final ok = await showDialog<bool>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: Text('Remove "${client.name}"?'),
        content: const Text('Its credentials are revoked. If the agent keeps running it will ask to pair again.'),
        actions: [
          TextButton(onPressed: () => Navigator.pop(ctx, false), child: const Text('Cancel')),
          FilledButton(
            style: FilledButton.styleFrom(backgroundColor: Palette.red),
            onPressed: () => Navigator.pop(ctx, true),
            child: const Text('Remove'),
          ),
        ],
      ),
    );
    if (ok == true && context.mounted) await guard(context, () => api.removeClient(client.id));
  }

  @override
  Widget build(BuildContext context) {
    return DragTarget<String>(
      onWillAcceptWithDetails: (d) => !client.assignedTimers.contains(d.data),
      onAcceptWithDetails: (d) =>
          guard(context, () => api.setAssignments(client.id, [...client.assignedTimers, d.data])),
      builder: (context, candidates, _) {
        final hovering = candidates.isNotEmpty;
        return AnimatedContainer(
          duration: const Duration(milliseconds: 120),
          margin: const EdgeInsets.only(bottom: 12),
          padding: const EdgeInsets.all(14),
          decoration: BoxDecoration(
            color: hovering ? const Color(0xFF2A2412) : Palette.card,
            borderRadius: BorderRadius.circular(12),
            border: Border.all(color: hovering ? Palette.amber : Palette.border),
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Row(
                children: [
                  _OnlineDot(client.online),
                  const SizedBox(width: 8),
                  Expanded(
                    child: Text(
                      client.name,
                      overflow: TextOverflow.ellipsis,
                      style: const TextStyle(fontWeight: FontWeight.w600),
                    ),
                  ),
                  for (final op in BulkOp.values)
                    IconButton(
                      tooltip: "${op.label} this client's timers",
                      visualDensity: VisualDensity.compact,
                      iconSize: 20,
                      color: op == BulkOp.start ? Palette.amber : Palette.text,
                      onPressed:
                          op.appliesToAny(client.assignedTimers, store.timers, (id) => store.runFor(client.id, id))
                          ? () => guard(context, () => api.bulkClient(client.id, op))
                          : null,
                      icon: Icon(op.icon),
                    ),
                  PopupMenuButton<String>(
                    tooltip: 'More',
                    padding: EdgeInsets.zero,
                    icon: const Icon(Icons.more_vert, size: 18, color: Palette.muted),
                    onSelected: (v) => switch (v) {
                      'assign' => _assign(context),
                      'rename' => _rename(context),
                      _ => _remove(context),
                    },
                    itemBuilder: (_) => const [
                      PopupMenuItem(value: 'assign', child: Text('Assign timers…')),
                      PopupMenuItem(value: 'rename', child: Text('Rename')),
                      PopupMenuItem(value: 'remove', child: Text('Remove')),
                    ],
                  ),
                ],
              ),
              Text(client.hostname, style: const TextStyle(color: Palette.muted, fontSize: 12)),
              const SizedBox(height: 10),
              if (client.assignedTimers.isEmpty)
                Text(
                  hovering ? 'Drop to assign' : 'No timers assigned',
                  style: const TextStyle(color: Palette.muted, fontSize: 12),
                ),
              Wrap(
                spacing: 6,
                runSpacing: 6,
                children: [
                  for (final id in client.assignedTimers)
                    _TimerChip(client: client, timerId: id, store: store, api: api),
                ],
              ),
              if (client.lastResult case final r?) _LastResult(result: r, timerName: store.timers[r.timerId]?.name),
            ],
          ),
        );
      },
    );
  }
}

/// One of a client's timers on its card. Tapping it offers Dismiss and +1 minute for this
/// client only; the delete button unassigns it.
class _TimerChip extends StatelessWidget {
  const _TimerChip({required this.client, required this.timerId, required this.store, required this.api});
  final ClientView client;
  final String timerId;
  final DeckStore store;
  final AdminApi api;

  Future<void> _actions(BuildContext context, TimerDef def, bool firing) async {
    final box = context.findRenderObject()! as RenderBox;
    final overlay = Overlay.of(context).context.findRenderObject()! as RenderBox;
    final position = RelativeRect.fromRect(
      Rect.fromPoints(
        box.localToGlobal(box.size.bottomLeft(Offset.zero), ancestor: overlay),
        box.localToGlobal(box.size.bottomRight(Offset.zero), ancestor: overlay),
      ),
      Offset.zero & overlay.size,
    );
    final op = await showMenu<String>(
      context: context,
      position: position,
      items: [
        PopupMenuItem(value: 'dismiss', enabled: firing, child: const Text('Dismiss')),
        PopupMenuItem(
          value: 'add',
          // Scheduled timers fire at a fixed time; they can only be snoozed once firing.
          enabled: firing || def.kind is! AtTime,
          child: Text(firing ? 'Snooze 1 minute' : 'Add 1 minute'),
        ),
      ],
    );
    if (op != null && context.mounted) {
      await guard(context, () => api.controlClient(client.id, timerId, op, addSecs: op == 'add' ? 60 : null));
    }
  }

  @override
  Widget build(BuildContext context) {
    final def = store.timers[timerId];
    final firing = store.runFor(client.id, timerId)?.state == RunState.firing;
    return InputChip(
      label: _ChipLabel(
        name: def?.name ?? 'unknown',
        separate: store.clientRuns[client.id]?[timerId],
        serverNow: store.clock.now,
      ),
      tooltip: def == null ? null : 'Dismiss or add time on this client',
      onPressed: def == null ? null : () => _actions(context, def, firing),
      visualDensity: VisualDensity.compact,
      backgroundColor: const Color(0xFF20242C),
      side: BorderSide(color: firing ? Palette.amber : Palette.border),
      deleteButtonTooltipMessage: 'Unassign',
      onDeleted: () => guard(
        context,
        () => api.setAssignments(client.id, client.assignedTimers.where((t) => t != timerId).toList()),
      ),
    );
  }
}

/// A timer's name on a client card, plus the state and time left of the
/// client's own copy of the run when it isn't following the shared one.
class _ChipLabel extends StatelessWidget {
  const _ChipLabel({required this.name, required this.separate, required this.serverNow});
  final String name;
  final TimerRun? separate;
  final DateTime Function() serverNow;

  @override
  Widget build(BuildContext context) {
    final run = separate;
    if (run == null) return Text(name, style: const TextStyle(fontSize: 12));
    final c = pillColors(run.state);
    return Tooltip(
      message: 'Running separately from other clients (${run.state.name})',
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Text(name, style: const TextStyle(fontSize: 12)),
          const SizedBox(width: 6),
          Container(
            padding: const EdgeInsets.symmetric(horizontal: 5, vertical: 1),
            decoration: BoxDecoration(color: c.bg, borderRadius: BorderRadius.circular(4)),
            child: Ticking(
              active: run.state == RunState.running,
              builder: (_) => Text(
                run.state == RunState.firing ? 'FIRING' : formatRemaining(run.remainingAt(serverNow())),
                style: TextStyle(color: c.fg, fontSize: 11, fontFamily: monoFamily, fontWeight: FontWeight.w700),
              ),
            ),
          ),
        ],
      ),
    );
  }
}

/// Bottom sheet listing every timer with a checkbox; pops the chosen timer ids
/// (in deck order) on Save.
class AssignSheet extends StatefulWidget {
  const AssignSheet({super.key, required this.client, required this.timers});
  final ClientView client;
  final List<TimerDef> timers;

  @override
  State<AssignSheet> createState() => _AssignSheetState();
}

class _AssignSheetState extends State<AssignSheet> {
  late final Set<String> _selected = {...widget.client.assignedTimers};

  @override
  Widget build(BuildContext context) {
    return SafeArea(
      child: ConstrainedBox(
        constraints: BoxConstraints(maxHeight: MediaQuery.sizeOf(context).height * 0.8),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(20, 0, 20, 8),
              child: Text(
                'Timers for ${widget.client.name}',
                style: const TextStyle(fontSize: 17, fontWeight: FontWeight.w600),
              ),
            ),
            if (widget.timers.isEmpty)
              const Padding(
                padding: EdgeInsets.all(20),
                child: Text('No timers yet.', style: TextStyle(color: Palette.muted)),
              ),
            Flexible(
              child: ListView(
                shrinkWrap: true,
                children: [
                  for (final t in widget.timers)
                    CheckboxListTile(
                      value: _selected.contains(t.id),
                      onChanged: (on) => setState(() => on! ? _selected.add(t.id) : _selected.remove(t.id)),
                      title: Text(t.name),
                      subtitle: Text(t.subtitle, style: const TextStyle(color: Palette.muted, fontSize: 12)),
                      controlAffinity: ListTileControlAffinity.leading,
                    ),
                ],
              ),
            ),
            Padding(
              padding: const EdgeInsets.all(16),
              child: Row(
                mainAxisAlignment: MainAxisAlignment.end,
                children: [
                  OutlinedButton(onPressed: () => Navigator.pop(context), child: const Text('Cancel')),
                  const SizedBox(width: 10),
                  FilledButton(
                    onPressed: () => Navigator.pop(context, [
                      for (final t in widget.timers)
                        if (_selected.contains(t.id)) t.id,
                    ]),
                    child: const Text('Save'),
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _LastResult extends StatelessWidget {
  const _LastResult({required this.result, required this.timerName});
  final ActionResult result;
  final String? timerName;

  @override
  Widget build(BuildContext context) {
    final t = result.at.toLocal();
    final time = '${t.hour.toString().padLeft(2, '0')}:${t.minute.toString().padLeft(2, '0')}';
    return Padding(
      padding: const EdgeInsets.only(top: 10),
      child: Tooltip(
        message: result.message,
        child: Row(
          children: [
            Icon(
              result.ok ? Icons.check_circle_outline : Icons.error_outline,
              size: 14,
              color: result.ok ? Palette.green : Palette.red,
            ),
            const SizedBox(width: 6),
            Expanded(
              child: Text(
                '$time ${timerName ?? 'test run'}: ${result.message}',
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: const TextStyle(color: Palette.muted, fontSize: 12),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
