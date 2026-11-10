// Mirrors of the Rust types in crates/core (JSON wire format).

import 'package:uuid/uuid.dart';

const _uuid = Uuid();
String newId() => _uuid.v4();

enum Weekday { mon, tue, wed, thu, fri, sat, sun }

extension WeekdayJson on Weekday {
  String get json => const ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'][index];
  String get short => json;
  static Weekday parse(String s) => Weekday.values[const ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'].indexOf(s)];
}

sealed class Repeat {
  const Repeat();
  Map<String, dynamic> toJson();
  String get label;

  static Repeat fromJson(Map<String, dynamic> j) => switch (j['type']) {
    'once' => const RepeatOnce(),
    'daily' => const RepeatDaily(),
    'weekdays' => const RepeatWeekdays(),
    'days' => RepeatDays((j['days'] as List).map((d) => WeekdayJson.parse(d as String)).toSet()),
    final t => throw FormatException('unknown repeat $t'),
  };
}

class RepeatOnce extends Repeat {
  const RepeatOnce();
  @override
  Map<String, dynamic> toJson() => {'type': 'once'};
  @override
  String get label => 'once';
}

class RepeatDaily extends Repeat {
  const RepeatDaily();
  @override
  Map<String, dynamic> toJson() => {'type': 'daily'};
  @override
  String get label => 'daily';
}

class RepeatWeekdays extends Repeat {
  const RepeatWeekdays();
  @override
  Map<String, dynamic> toJson() => {'type': 'weekdays'};
  @override
  String get label => 'weekdays';
}

class RepeatDays extends Repeat {
  const RepeatDays(this.days);
  final Set<Weekday> days;
  @override
  Map<String, dynamic> toJson() => {
    'type': 'days',
    'days': (days.toList()..sort((a, b) => a.index.compareTo(b.index))).map((d) => d.json).toList(),
  };
  @override
  String get label => (days.toList()..sort((a, b) => a.index.compareTo(b.index))).map((d) => d.short).join(' ');
}

sealed class TimerKind {
  const TimerKind();
  Map<String, dynamic> toJson();

  static TimerKind fromJson(Map<String, dynamic> j) => switch (j['type']) {
    'countdown' => Countdown(Duration(seconds: j['duration_secs'] as int)),
    'at_time' => AtTime(_parseHhmm(j['time'] as String), Repeat.fromJson(j['repeat'] as Map<String, dynamic>)),
    final t => throw FormatException('unknown timer kind $t'),
  };
}

class Countdown extends TimerKind {
  const Countdown(this.duration);
  final Duration duration;
  @override
  Map<String, dynamic> toJson() => {'type': 'countdown', 'duration_secs': duration.inSeconds};
}

/// Wall-clock time of day (hour, minute).
typedef TimeOfDayHm = ({int hour, int minute});

TimeOfDayHm _parseHhmm(String s) {
  final parts = s.split(':');
  return (hour: int.parse(parts[0]), minute: int.parse(parts[1]));
}

String formatHm(TimeOfDayHm t) => '${t.hour.toString().padLeft(2, '0')}:${t.minute.toString().padLeft(2, '0')}';

class AtTime extends TimerKind {
  const AtTime(this.time, this.repeat);
  final TimeOfDayHm time;
  final Repeat repeat;
  @override
  Map<String, dynamic> toJson() => {'type': 'at_time', 'time': formatHm(time), 'repeat': repeat.toJson()};
}

enum ActionMode { parallel, afterPrevious }

sealed class ActionKind {
  const ActionKind();
  Map<String, dynamic> toJson();

  static ActionKind fromJson(Map<String, dynamic> j) => switch (j['type']) {
    'sound' => SoundAction(
      sound: j['sound'] as String,
      volume: (j['volume'] as num).toDouble(),
      loopUntilDismissed: j['loop_until_dismissed'] as bool,
    ),
    'command' => CommandAction(j['cmd'] as String),
    final t => throw FormatException('unknown action $t'),
  };
}

class SoundAction extends ActionKind {
  const SoundAction({required this.sound, this.volume = 0.8, this.loopUntilDismissed = true});
  final String sound;
  final double volume;
  final bool loopUntilDismissed;
  @override
  Map<String, dynamic> toJson() => {
    'type': 'sound',
    'sound': sound,
    'volume': volume,
    'loop_until_dismissed': loopUntilDismissed,
  };
}

class CommandAction extends ActionKind {
  const CommandAction(this.cmd);
  final String cmd;
  @override
  Map<String, dynamic> toJson() => {'type': 'command', 'cmd': cmd};
}

/// Sounds every agent can play without a file path.
const bundledSounds = ['gong', 'chime', 'bell'];

class TimerAction {
  TimerAction({String? id, required this.kind, this.mode = ActionMode.parallel}) : id = id ?? newId();
  final String id;
  final ActionKind kind;
  final ActionMode mode;

  TimerAction copyWith({ActionKind? kind, ActionMode? mode}) =>
      TimerAction(id: id, kind: kind ?? this.kind, mode: mode ?? this.mode);

  factory TimerAction.fromJson(Map<String, dynamic> j) => TimerAction(
    id: j['id'] as String,
    kind: ActionKind.fromJson(j['kind'] as Map<String, dynamic>),
    mode: j['mode'] == 'after_previous' ? ActionMode.afterPrevious : ActionMode.parallel,
  );

  Map<String, dynamic> toJson() => {
    'id': id,
    'kind': kind.toJson(),
    'mode': mode == ActionMode.afterPrevious ? 'after_previous' : 'parallel',
  };
}

class TimerDef {
  const TimerDef({
    required this.id,
    required this.name,
    required this.kind,
    this.actions = const [],
    this.allowClientDismiss = true,
  });
  final String id;
  final String name;
  final TimerKind kind;
  final List<TimerAction> actions;

  /// Whether client displays may dismiss the timer while it is firing.
  final bool allowClientDismiss;

  factory TimerDef.fromJson(Map<String, dynamic> j) => TimerDef(
    id: j['id'] as String,
    name: j['name'] as String,
    kind: TimerKind.fromJson(j['kind'] as Map<String, dynamic>),
    actions: [for (final a in (j['actions'] as List? ?? [])) TimerAction.fromJson(a as Map<String, dynamic>)],
    allowClientDismiss: j['allow_client_dismiss'] as bool? ?? true,
  );

  /// Body for create/update requests (the server owns the id).
  Map<String, dynamic> toDraftJson() => {
    'name': name,
    'kind': kind.toJson(),
    'actions': [for (final a in actions) a.toJson()],
    'allow_client_dismiss': allowClientDismiss,
  };

  Map<String, dynamic> toJson() => {'id': id, ...toDraftJson()};

  /// Subtitle such as "Countdown · 15:00" or "At 09:45 · weekdays".
  String get subtitle => switch (kind) {
    Countdown(:final duration) => 'Countdown · ${formatClock(duration)}',
    AtTime(:final time, :final repeat) => 'At ${formatHm(time)} · ${repeat.label}',
  };
}

enum RunState { idle, running, paused, firing }

/// An operation the control deck applies to many timers at once. Mirrors the
/// server's rules so buttons can be disabled when nothing would change.
enum BulkOp {
  start('Start'),
  pause('Pause'),
  reset('Reset');

  const BulkOp(this.label);
  final String label;

  bool appliesTo(TimerDef def, TimerRun? run) {
    final state = run?.state ?? RunState.idle;
    return switch (this) {
      BulkOp.start => state == RunState.idle || state == RunState.paused,
      BulkOp.pause => state == RunState.running && def.kind is Countdown,
      BulkOp.reset =>
        !(state == RunState.idle &&
            (run == null ||
                run.remainingMs ==
                    switch (def.kind) {
                      Countdown(:final duration) => duration.inMilliseconds,
                      AtTime() => 0,
                    })),
    };
  }

  /// Whether this applies to any of [timerIds].
  bool appliesToAny(Iterable<String> timerIds, Map<String, TimerDef> timers, Map<String, TimerRun> runs) =>
      timerIds.any((id) => timers[id] != null && appliesTo(timers[id]!, runs[id]));
}

class TimerRun {
  const TimerRun({
    required this.runId,
    required this.timerId,
    required this.state,
    required this.endsAt,
    required this.remainingMs,
    required this.totalMs,
  });
  final String runId;
  final String timerId;
  final RunState state;
  final DateTime? endsAt;
  final int remainingMs;
  final int totalMs;

  factory TimerRun.fromJson(Map<String, dynamic> j) => TimerRun(
    runId: j['run_id'] as String,
    timerId: j['timer_id'] as String,
    state: RunState.values.byName(j['state'] as String),
    endsAt: j['ends_at'] == null ? null : DateTime.parse(j['ends_at'] as String),
    remainingMs: j['remaining_ms'] as int,
    totalMs: j['total_ms'] as int,
  );

  /// Time left given the current server time.
  Duration remainingAt(DateTime serverNow) {
    if (state == RunState.running && endsAt != null) {
      final d = endsAt!.difference(serverNow);
      return d.isNegative ? Duration.zero : d;
    }
    return Duration(milliseconds: remainingMs);
  }

  /// Fraction of time remaining, 0..1.
  double fractionAt(DateTime serverNow) {
    if (totalMs <= 0) return state == RunState.firing ? 1 : 0;
    if (state == RunState.firing) return 1;
    return (remainingAt(serverNow).inMilliseconds / totalMs).clamp(0.0, 1.0);
  }
}

enum ClientStatus { pending, approved }

class ActionResult {
  const ActionResult({
    required this.timerId,
    required this.actionId,
    required this.ok,
    required this.exitCode,
    required this.message,
    required this.at,
  });
  final String? timerId;
  final String actionId;
  final bool ok;
  final int? exitCode;
  final String message;
  final DateTime at;

  factory ActionResult.fromJson(Map<String, dynamic> j) => ActionResult(
    timerId: j['timer_id'] as String?,
    actionId: j['action_id'] as String,
    ok: j['ok'] as bool,
    exitCode: j['exit_code'] as int?,
    message: j['message'] as String,
    at: DateTime.parse(j['at'] as String),
  );
}

class ClientView {
  const ClientView({
    required this.id,
    required this.name,
    required this.hostname,
    required this.status,
    required this.online,
    required this.assignedTimers,
    this.lastResult,
  });
  final String id;
  final String name;
  final String hostname;
  final ClientStatus status;
  final bool online;
  final List<String> assignedTimers;
  final ActionResult? lastResult;

  factory ClientView.fromJson(Map<String, dynamic> j) => ClientView(
    id: j['id'] as String,
    name: j['name'] as String,
    hostname: j['hostname'] as String,
    status: ClientStatus.values.byName(j['status'] as String),
    online: j['online'] as bool,
    assignedTimers: [for (final t in j['assigned_timers'] as List) t as String],
    lastResult: j['last_result'] == null ? null : ActionResult.fromJson(j['last_result'] as Map<String, dynamic>),
  );
}

/// `mm:ss`, or `h:mm:ss` from one hour up.
String formatClock(Duration d) {
  final h = d.inHours;
  final m = d.inMinutes.remainder(60).toString().padLeft(2, '0');
  final s = d.inSeconds.remainder(60).toString().padLeft(2, '0');
  return h > 0 ? '$h:$m:$s' : '$m:$s';
}

/// Compact display for long waits, e.g. "3h 14m" (the mockup's alarm card).
String formatRemaining(Duration d) {
  // Round up so a timer never shows 00:00 before it actually fires.
  final up = Duration(seconds: (d.inMilliseconds / 1000).ceil());
  if (up.inHours >= 1) return '${up.inHours}h ${up.inMinutes.remainder(60).toString().padLeft(2, '0')}m';
  return formatClock(up);
}
