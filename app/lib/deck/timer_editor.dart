import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../models.dart';
import '../theme.dart';

enum _Type { countdown, atTime }

enum _RepeatChoice { once, daily, weekdays, days }

/// Create/edit dialog from docs/design "configure idea": when on the left, actions on the right.
class TimerEditor extends StatefulWidget {
  const TimerEditor({super.key, this.initial, required this.clients, required this.onTestRun});
  final TimerDef? initial;
  final List<ClientView> clients;
  final Future<void> Function(String clientId, List<TimerAction> actions) onTestRun;

  @override
  State<TimerEditor> createState() => _TimerEditorState();
}

class _TimerEditorState extends State<TimerEditor> {
  late final _name = TextEditingController(text: widget.initial?.name ?? '');
  _Type _type = _Type.countdown;
  late final _h = TextEditingController();
  late final _m = TextEditingController();
  late final _s = TextEditingController();
  late final _atH = TextEditingController(text: '09');
  late final _atM = TextEditingController(text: '00');
  _RepeatChoice _repeat = _RepeatChoice.weekdays;
  Set<Weekday> _days = {Weekday.mon};
  late final List<TimerAction> _actions = [...?widget.initial?.actions];
  late bool _allowClientDismiss = widget.initial?.allowClientDismiss ?? true;
  String? _error;
  String? _testStatus;

  /// Phone layout: full screen with WHEN / THEN tabs instead of side-by-side panes.
  bool _compact = false;

  @override
  void initState() {
    super.initState();
    switch (widget.initial?.kind) {
      case Countdown(:final duration):
        _setDuration(duration);
      case AtTime(:final time, :final repeat):
        _type = _Type.atTime;
        _atH.text = time.hour.toString().padLeft(2, '0');
        _atM.text = time.minute.toString().padLeft(2, '0');
        _repeat = switch (repeat) {
          RepeatOnce() => _RepeatChoice.once,
          RepeatDaily() => _RepeatChoice.daily,
          RepeatWeekdays() => _RepeatChoice.weekdays,
          RepeatDays() => _RepeatChoice.days,
        };
        if (repeat is RepeatDays) _days = {...repeat.days};
      case null:
        _setDuration(const Duration(minutes: 15));
    }
  }

  @override
  void dispose() {
    for (final c in [_name, _h, _m, _s, _atH, _atM]) {
      c.dispose();
    }
    super.dispose();
  }

  void _setDuration(Duration d) {
    _h.text = d.inHours.toString().padLeft(2, '0');
    _m.text = d.inMinutes.remainder(60).toString().padLeft(2, '0');
    _s.text = d.inSeconds.remainder(60).toString().padLeft(2, '0');
  }

  int _num(TextEditingController c) => int.tryParse(c.text) ?? 0;

  /// Validate and build the definition, or set [_error].
  TimerDef? _build() {
    String? err;
    final name = _name.text.trim();
    late TimerKind kind;
    if (name.isEmpty) err = 'Give the timer a name.';
    if (_type == _Type.countdown) {
      final d = Duration(hours: _num(_h), minutes: _num(_m), seconds: _num(_s));
      if (d == Duration.zero) err ??= 'Duration must be longer than zero.';
      kind = Countdown(d);
    } else {
      final (h, m) = (_num(_atH), _num(_atM));
      if (h > 23 || m > 59) err ??= 'Time must be between 00:00 and 23:59.';
      if (_repeat == _RepeatChoice.days && _days.isEmpty) err ??= 'Pick at least one day.';
      kind = AtTime(
        (hour: h, minute: m),
        switch (_repeat) {
          _RepeatChoice.once => const RepeatOnce(),
          _RepeatChoice.daily => const RepeatDaily(),
          _RepeatChoice.weekdays => const RepeatWeekdays(),
          _RepeatChoice.days => RepeatDays({..._days}),
        },
      );
    }
    for (final a in _actions) {
      switch (a.kind) {
        case CommandAction(:final cmd) when cmd.trim().isEmpty:
          err ??= 'Commands cannot be empty.';
        case SoundAction(:final sound) when !bundledSounds.contains(sound) && !sound.startsWith('/'):
          err ??= 'Custom sounds need an absolute file path on the client.';
        default:
      }
    }
    setState(() => _error = err);
    if (err != null) return null;
    return TimerDef(
      id: widget.initial?.id ?? '',
      name: name,
      kind: kind,
      actions: _actions,
      allowClientDismiss: _allowClientDismiss,
    );
  }

  Future<void> _testRun(ClientView client) async {
    setState(() => _testStatus = 'Sending to ${client.name}…');
    try {
      await widget.onTestRun(client.id, _actions);
      setState(() => _testStatus = 'Sent to ${client.name}. Result appears on its card.');
    } on Object catch (e) {
      setState(() => _testStatus = 'Test run failed: $e');
    }
  }

  void _update(int i, TimerAction a) => setState(() => _actions[i] = a);

  void _save() {
    final def = _build();
    if (def != null) Navigator.pop(context, def);
  }

  @override
  Widget build(BuildContext context) {
    _compact = MediaQuery.sizeOf(context).width < compactBreakpoint;
    if (_compact) {
      return Dialog.fullscreen(
        child: DefaultTabController(
          length: 2,
          child: Scaffold(
            appBar: AppBar(
              leading: IconButton(
                icon: const Icon(Icons.close),
                tooltip: 'Cancel',
                onPressed: () => Navigator.pop(context),
              ),
              title: Text(widget.initial == null ? 'New timer' : 'Edit timer'),
              actions: [
                TextButton(onPressed: _save, child: const Text('Save')),
                const SizedBox(width: 8),
              ],
              bottom: PreferredSize(
                preferredSize: Size.fromHeight(kTextTabBarHeight + (_error == null ? 0 : 32)),
                child: Column(
                  children: [
                    // Shown above the tabs so it is visible whichever tab has the problem.
                    if (_error != null)
                      SizedBox(
                        height: 32,
                        child: Center(
                          child: Text(_error!, style: const TextStyle(color: Palette.red)),
                        ),
                      ),
                    const TabBar(
                      tabs: [
                        Tab(text: '1 · When'),
                        Tab(text: '2 · Then do'),
                      ],
                    ),
                  ],
                ),
              ),
            ),
            body: TabBarView(children: [_whenPane(), _thenPane()]),
          ),
        ),
      );
    }
    return Dialog(
      insetPadding: const EdgeInsets.all(24),
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(24),
        side: const BorderSide(color: Palette.border),
      ),
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 1000, maxHeight: 760),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            SizedBox(width: 400, child: _whenPane()),
            const VerticalDivider(width: 1),
            Expanded(child: _thenPane()),
          ],
        ),
      ),
    );
  }

  Widget _label(String text) => Padding(
    padding: const EdgeInsets.only(top: 20, bottom: 8),
    child: Text(text, style: const TextStyle(color: Palette.muted, fontSize: 13)),
  );

  Widget _heading(String step, String title) => Column(
    crossAxisAlignment: CrossAxisAlignment.start,
    children: [
      Text(step, style: const TextStyle(color: Palette.muted, fontSize: 12, letterSpacing: 1)),
      const SizedBox(height: 4),
      Text(title, style: const TextStyle(fontSize: 22, fontWeight: FontWeight.w600)),
    ],
  );

  Widget _whenPane() {
    Widget typeButton(String label, _Type? t) {
      final selected = t != null && _type == t;
      return Expanded(
        child: Tooltip(
          message: t == null ? 'Coming soon' : '',
          child: OutlinedButton(
            onPressed: t == null ? null : () => setState(() => _type = t),
            style: OutlinedButton.styleFrom(
              backgroundColor: selected ? const Color(0xFF2A2412) : null,
              side: BorderSide(color: selected ? Palette.amber : Palette.border),
            ),
            child: Text(label),
          ),
        ),
      );
    }

    return ListView(
      padding: EdgeInsets.all(_compact ? 20 : 32),
      children: [
        _heading('1 · WHEN', widget.initial == null ? 'New timer' : 'Edit timer'),
        _label('Name'),
        TextField(controller: _name, autofocus: widget.initial == null),
        _label('Type'),
        Row(
          children: [
            typeButton('Countdown', _Type.countdown),
            const SizedBox(width: 8),
            typeButton('At a time', _Type.atTime),
          ],
        ),
        const SizedBox(height: 8),
        Row(children: [typeButton('Interval', null), const SizedBox(width: 8), typeButton('Sequence', null)]),
        if (_type == _Type.countdown) ...[
          _label('Duration'),
          _DigitsRow(controllers: [_h, _m, _s], maxes: const [99, 59, 59]),
          const SizedBox(height: 10),
          Wrap(
            spacing: 8,
            children: [
              for (final (label, d) in const [
                ('5m', Duration(minutes: 5)),
                ('15m', Duration(minutes: 15)),
                ('25m', Duration(minutes: 25)),
                ('1h', Duration(hours: 1)),
              ])
                ActionChip(
                  label: Text(label, style: const TextStyle(fontSize: 12)),
                  visualDensity: VisualDensity.compact,
                  onPressed: () => setState(() => _setDuration(d)),
                ),
            ],
          ),
        ] else ...[
          _label('Time (server local time)'),
          _DigitsRow(controllers: [_atH, _atM], maxes: const [23, 59]),
          _label('Repeat'),
          DropdownButtonFormField<_RepeatChoice>(
            initialValue: _repeat,
            items: const [
              DropdownMenuItem(value: _RepeatChoice.once, child: Text('Once')),
              DropdownMenuItem(value: _RepeatChoice.daily, child: Text('Every day')),
              DropdownMenuItem(value: _RepeatChoice.weekdays, child: Text('Weekdays')),
              DropdownMenuItem(value: _RepeatChoice.days, child: Text('Specific days')),
            ],
            onChanged: (v) => setState(() => _repeat = v!),
          ),
          if (_repeat == _RepeatChoice.days) ...[
            const SizedBox(height: 10),
            Wrap(
              spacing: 6,
              runSpacing: 6,
              children: [
                for (final d in Weekday.values)
                  FilterChip(
                    label: Text(d.short, style: const TextStyle(fontSize: 12)),
                    selected: _days.contains(d),
                    onSelected: (on) => setState(() => on ? _days.add(d) : _days.remove(d)),
                  ),
              ],
            ),
          ],
        ],
        _label('Client display'),
        CheckboxListTile(
          value: _allowClientDismiss,
          onChanged: (v) => setState(() => _allowClientDismiss = v!),
          dense: true,
          contentPadding: EdgeInsets.zero,
          controlAffinity: ListTileControlAffinity.leading,
          title: const Text('Clients can dismiss', style: TextStyle(fontSize: 13)),
          subtitle: const Text(
            'When off, the timer returns to idle once every client finishes its actions. '
            'The control deck can still dismiss it early.',
            style: TextStyle(color: Palette.muted, fontSize: 12),
          ),
        ),
      ],
    );
  }

  Widget _thenPane() {
    final canTest = widget.clients.isNotEmpty && _actions.isNotEmpty;
    return Padding(
      padding: EdgeInsets.all(_compact ? 20 : 32),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            children: [
              Expanded(child: _heading('2 · THEN DO', 'On finish')),
              Tooltip(
                message: widget.clients.isEmpty ? 'No approved client is online' : 'Run these actions on a client now',
                child: PopupMenuButton<ClientView>(
                  enabled: canTest,
                  onSelected: _testRun,
                  itemBuilder: (_) => [for (final c in widget.clients) PopupMenuItem(value: c, child: Text(c.name))],
                  child: IgnorePointer(
                    child: OutlinedButton(onPressed: canTest ? () {} : null, child: const Text('▶ Test run')),
                  ),
                ),
              ),
            ],
          ),
          if (_testStatus != null)
            Padding(
              padding: const EdgeInsets.only(top: 8),
              child: Text(
                _testStatus!,
                textAlign: TextAlign.end,
                style: const TextStyle(color: Palette.muted, fontSize: 12),
              ),
            ),
          const SizedBox(height: 16),
          Expanded(
            child: ListView(
              children: [
                if (_actions.isEmpty)
                  const Padding(
                    padding: EdgeInsets.only(bottom: 16),
                    child: Text(
                      'Actions run on every client this timer is assigned to.',
                      style: TextStyle(color: Palette.muted),
                    ),
                  ),
                for (final (i, a) in _actions.indexed)
                  _ActionStep(
                    key: ValueKey(a.id),
                    index: i,
                    action: a,
                    isLast: i == _actions.length - 1,
                    canLoop: _allowClientDismiss,
                    compact: _compact,
                    onChanged: (a) => _update(i, a),
                    onRemove: () => setState(() => _actions.removeAt(i)),
                    onMoveUp: i == 0 ? null : () => setState(() => _actions.insert(i - 1, _actions.removeAt(i))),
                  ),
                Padding(
                  padding: const EdgeInsets.only(left: 44),
                  child: Wrap(
                    spacing: 8,
                    runSpacing: 8,
                    children: [
                      _AddButton(
                        '+ Sound',
                        () => setState(() => _actions.add(TimerAction(kind: const SoundAction(sound: 'gong')))),
                      ),
                      _AddButton(
                        '+ Command',
                        () => setState(
                          () =>
                              _actions.add(TimerAction(kind: const CommandAction(''), mode: ActionMode.afterPrevious)),
                        ),
                      ),
                    ],
                  ),
                ),
              ],
            ),
          ),
          // The compact layout has Cancel / Save and the error in its app bar.
          if (_error != null && !_compact)
            Padding(
              padding: const EdgeInsets.only(bottom: 12),
              child: Text(_error!, style: const TextStyle(color: Palette.red)),
            ),
          if (!_compact)
            Row(
              mainAxisAlignment: MainAxisAlignment.end,
              children: [
                OutlinedButton(onPressed: () => Navigator.pop(context), child: const Text('Cancel')),
                const SizedBox(width: 10),
                FilledButton(onPressed: _save, child: const Text('Save timer')),
              ],
            ),
        ],
      ),
    );
  }
}

class _AddButton extends StatelessWidget {
  const _AddButton(this.label, this.onPressed);
  final String label;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) => OutlinedButton(
    onPressed: onPressed,
    style: OutlinedButton.styleFrom(
      side: const BorderSide(color: Palette.border, style: BorderStyle.solid),
      foregroundColor: Palette.muted,
    ),
    child: Text(label, style: const TextStyle(fontSize: 13)),
  );
}

/// Big two-digit boxes separated by colons (duration or time of day).
class _DigitsRow extends StatelessWidget {
  const _DigitsRow({required this.controllers, required this.maxes});
  final List<TextEditingController> controllers;
  final List<int> maxes;

  @override
  Widget build(BuildContext context) {
    final children = <Widget>[];
    for (final (i, c) in controllers.indexed) {
      if (i > 0) {
        children.add(
          const Padding(
            padding: EdgeInsets.symmetric(horizontal: 10),
            child: Text(
              ':',
              style: TextStyle(fontSize: 30, fontWeight: FontWeight.w700, color: Palette.muted),
            ),
          ),
        );
      }
      children.add(
        SizedBox(
          width: 76,
          child: TextField(
            controller: c,
            textAlign: TextAlign.center,
            keyboardType: TextInputType.number,
            inputFormatters: [
              FilteringTextInputFormatter.digitsOnly,
              LengthLimitingTextInputFormatter(2),
              _MaxValue(maxes[i]),
            ],
            style: const TextStyle(fontFamily: monoFamily, fontSize: 34, fontWeight: FontWeight.w700),
            decoration: const InputDecoration(contentPadding: EdgeInsets.symmetric(vertical: 14)),
          ),
        ),
      );
    }
    return Row(children: children);
  }
}

class _MaxValue extends TextInputFormatter {
  _MaxValue(this.max);
  final int max;

  @override
  TextEditingValue formatEditUpdate(TextEditingValue oldValue, TextEditingValue newValue) {
    final v = int.tryParse(newValue.text);
    return v != null && v > max ? oldValue : newValue;
  }
}

/// One numbered step in the "On finish" timeline.
class _ActionStep extends StatelessWidget {
  const _ActionStep({
    super.key,
    required this.index,
    required this.action,
    required this.isLast,
    required this.canLoop,
    required this.compact,
    required this.onChanged,
    required this.onRemove,
    required this.onMoveUp,
  });
  final int index;
  final TimerAction action;
  final bool isLast;
  final bool canLoop;
  final bool compact;
  final ValueChanged<TimerAction> onChanged;
  final VoidCallback onRemove;
  final VoidCallback? onMoveUp;

  @override
  Widget build(BuildContext context) {
    final isSound = action.kind is SoundAction;
    // The connector line is drawn behind the row rather than sized with
    // IntrinsicHeight, which misjudges the fields' height on narrow widths.
    return Stack(
      children: [
        Positioned(left: 14, top: 32, bottom: 4, child: Container(width: 2, color: Palette.border)),
        Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            SizedBox(
              width: 30,
              child: CircleAvatar(
                radius: 14,
                backgroundColor: const Color(0xFF262A33),
                child: Text('${index + 1}', style: const TextStyle(fontSize: 12, color: Palette.text)),
              ),
            ),
            const SizedBox(width: 14),
            Expanded(
              child: Container(
                margin: const EdgeInsets.only(bottom: 14),
                padding: const EdgeInsets.fromLTRB(16, 10, 8, 14),
                decoration: BoxDecoration(
                  color: Palette.field,
                  borderRadius: BorderRadius.circular(14),
                  border: Border.all(color: Palette.border),
                ),
                child: Material(
                  // Gives the checkbox tile below a surface for its ink.
                  type: MaterialType.transparency,
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      Row(
                        children: [
                          Text(isSound ? '♪' : r'$', style: const TextStyle(color: Palette.amber)),
                          const SizedBox(width: 8),
                          Expanded(
                            child: Text(
                              isSound ? 'Play sound' : 'Run command',
                              maxLines: 1,
                              overflow: TextOverflow.ellipsis,
                              style: const TextStyle(fontWeight: FontWeight.w600),
                            ),
                          ),
                          if (index > 0)
                            Tooltip(
                              message: action.mode == ActionMode.parallel
                                  ? 'Starts together with the previous step'
                                  : 'Starts after the previous step finishes',
                              child: TextButton(
                                onPressed: () => onChanged(
                                  action.copyWith(
                                    mode: action.mode == ActionMode.parallel
                                        ? ActionMode.afterPrevious
                                        : ActionMode.parallel,
                                  ),
                                ),
                                child: Text(
                                  action.mode == ActionMode.parallel ? 'parallel' : 'after $index',
                                  style: const TextStyle(color: Palette.muted, fontSize: 12),
                                ),
                              ),
                            ),
                          IconButton(
                            onPressed: onMoveUp,
                            tooltip: 'Move up',
                            visualDensity: VisualDensity.compact,
                            icon: const Icon(Icons.arrow_upward, size: 16),
                          ),
                          IconButton(
                            onPressed: onRemove,
                            tooltip: 'Remove',
                            visualDensity: VisualDensity.compact,
                            icon: const Icon(Icons.close, size: 16),
                          ),
                        ],
                      ),
                      const SizedBox(height: 6),
                      switch (action.kind) {
                        final SoundAction s => _SoundFields(
                          sound: s,
                          canLoop: canLoop,
                          compact: compact,
                          onChanged: (k) => onChanged(action.copyWith(kind: k)),
                        ),
                        CommandAction(:final cmd) => Padding(
                          padding: const EdgeInsets.only(right: 8),
                          child: TextFormField(
                            initialValue: cmd,
                            autofocus: cmd.isEmpty,
                            style: const TextStyle(fontFamily: monoFamily, fontSize: 13),
                            decoration: const InputDecoration(hintText: 'e.g. loginctl lock-session'),
                            onChanged: (v) => onChanged(action.copyWith(kind: CommandAction(v))),
                          ),
                        ),
                      },
                    ],
                  ),
                ),
              ),
            ),
          ],
        ),
      ],
    );
  }
}

class _SoundFields extends StatelessWidget {
  const _SoundFields({required this.sound, required this.canLoop, required this.compact, required this.onChanged});
  final SoundAction sound;

  /// False when clients cannot dismiss the timer: the sound then plays once so
  /// the run can finish on its own.
  final bool canLoop;

  /// Narrow screens put the volume slider on its own line.
  final bool compact;
  final ValueChanged<SoundAction> onChanged;

  static const _custom = '__custom__';

  SoundAction _with({String? name, double? volume, bool? loop}) => SoundAction(
    sound: name ?? sound.sound,
    volume: volume ?? sound.volume,
    loopUntilDismissed: loop ?? sound.loopUntilDismissed,
  );

  @override
  Widget build(BuildContext context) {
    final isCustom = !bundledSounds.contains(sound.sound);
    final picker = DropdownButtonFormField<String>(
      initialValue: isCustom ? _custom : sound.sound,
      items: [
        for (final s in bundledSounds) DropdownMenuItem(value: s, child: Text(s)),
        const DropdownMenuItem(value: _custom, child: Text('file…')),
      ],
      onChanged: (v) => onChanged(_with(name: v == _custom ? '' : v)),
    );
    final volume = Row(
      children: [
        const Icon(Icons.volume_up_outlined, size: 16, color: Palette.muted),
        Expanded(
          child: Slider(
            value: sound.volume,
            onChanged: (v) => onChanged(_with(volume: (v * 20).round() / 20)),
          ),
        ),
        Text('${(sound.volume * 100).round()}%', style: const TextStyle(color: Palette.muted, fontSize: 12)),
      ],
    );
    return Padding(
      padding: const EdgeInsets.only(right: 8),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (compact) ...[
            picker,
            volume,
          ] else
            Row(
              children: [
                SizedBox(width: 150, child: picker),
                const SizedBox(width: 12),
                Expanded(child: volume),
              ],
            ),
          if (isCustom) ...[
            const SizedBox(height: 8),
            TextFormField(
              initialValue: sound.sound,
              style: const TextStyle(fontFamily: monoFamily, fontSize: 13),
              decoration: const InputDecoration(hintText: 'Absolute path on the client, e.g. /usr/share/sounds/…'),
              onChanged: (v) => onChanged(_with(name: v)),
            ),
          ],
          CheckboxListTile(
            value: canLoop && sound.loopUntilDismissed,
            onChanged: canLoop ? (v) => onChanged(_with(loop: v)) : null,
            dense: true,
            contentPadding: EdgeInsets.zero,
            controlAffinity: ListTileControlAffinity.leading,
            title: const Text('Loop until dismissed', style: TextStyle(fontSize: 13)),
            subtitle: canLoop
                ? null
                : const Text(
                    'Plays once: clients cannot dismiss this timer',
                    style: TextStyle(color: Palette.muted, fontSize: 12),
                  ),
          ),
        ],
      ),
    );
  }
}
