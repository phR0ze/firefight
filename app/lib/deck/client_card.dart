import 'package:flutter/material.dart';

import '../api/admin_api.dart';
import '../api/stores.dart';
import '../models.dart';
import '../theme.dart';
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
                      tooltip: '${op.label} all assigned timers',
                      visualDensity: VisualDensity.compact,
                      iconSize: 20,
                      color: op == BulkOp.start ? Palette.amber : Palette.text,
                      onPressed: op.appliesToAny(client.assignedTimers, store.timers, store.runs)
                          ? () => guard(context, () => api.bulkClient(client.id, op))
                          : null,
                      icon: Icon(op.icon),
                    ),
                  PopupMenuButton<String>(
                    tooltip: 'More',
                    padding: EdgeInsets.zero,
                    icon: const Icon(Icons.more_vert, size: 18, color: Palette.muted),
                    onSelected: (v) => v == 'rename' ? _rename(context) : _remove(context),
                    itemBuilder: (_) => const [
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
                    InputChip(
                      label: Text(store.timers[id]?.name ?? 'unknown', style: const TextStyle(fontSize: 12)),
                      visualDensity: VisualDensity.compact,
                      backgroundColor: const Color(0xFF20242C),
                      side: const BorderSide(color: Palette.border),
                      deleteButtonTooltipMessage: 'Unassign',
                      onDeleted: () => guard(
                        context,
                        () => api.setAssignments(client.id, client.assignedTimers.where((t) => t != id).toList()),
                      ),
                    ),
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
