import 'dart:convert';

import 'package:firefight/models.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  // Shape produced by serde for crates/core TimerDef.
  const serverJson = '''
  {
    "id": "8f0c3a8e-1f7a-4b7e-9c55-0a4f1c2b3d4e",
    "name": "Stand-up",
    "kind": {"type": "at_time", "time": "09:45", "repeat": {"type": "days", "days": ["Mon", "Wed"]}},
    "actions": [
      {"id": "a1", "kind": {"type": "sound", "sound": "gong", "volume": 0.8, "loop_until_dismissed": true}, "mode": "parallel"},
      {"id": "a2", "kind": {"type": "command", "cmd": "loginctl lock-session"}, "mode": "after_previous"}
    ]
  }''';

  test('TimerDef parses server JSON and round-trips', () {
    final def = TimerDef.fromJson(jsonDecode(serverJson) as Map<String, dynamic>);
    expect(def.name, 'Stand-up');
    final kind = def.kind as AtTime;
    expect(kind.time, (hour: 9, minute: 45));
    expect((kind.repeat as RepeatDays).days, {Weekday.mon, Weekday.wed});
    expect(def.actions[1].mode, ActionMode.afterPrevious);
    expect(def.subtitle, 'At 09:45 · Mon Wed');

    final again = TimerDef.fromJson(jsonDecode(jsonEncode(def.toJson())) as Map<String, dynamic>);
    expect(jsonEncode(again.toJson()), jsonEncode(def.toJson()));
  });

  test('countdown draft JSON matches the server API', () {
    final def = TimerDef(id: '', name: 'Tea', kind: const Countdown(Duration(minutes: 4)));
    expect(def.toDraftJson(), {
      'name': 'Tea',
      'kind': {'type': 'countdown', 'duration_secs': 240},
      'actions': <dynamic>[],
      'allow_client_dismiss': true,
    });
  });

  test('TimerRun remaining and fraction', () {
    final now = DateTime.utc(2026, 11, 9, 12);
    final run = TimerRun.fromJson({
      'run_id': 'r',
      'timer_id': 't',
      'state': 'running',
      'ends_at': now.add(const Duration(seconds: 30)).toIso8601String(),
      'remaining_ms': 60000,
      'total_ms': 60000,
    });
    expect(run.remainingAt(now), const Duration(seconds: 30));
    expect(run.fractionAt(now), closeTo(0.5, 1e-9));
    expect(run.remainingAt(now.add(const Duration(minutes: 5))), Duration.zero);
  });

  test('time formatting', () {
    expect(formatClock(const Duration(minutes: 15)), '15:00');
    expect(formatClock(const Duration(hours: 1, minutes: 2, seconds: 3)), '1:02:03');
    expect(formatRemaining(const Duration(hours: 3, minutes: 14)), '3h 14m');
    // Rounds up so the display never shows 00:00 early.
    expect(formatRemaining(const Duration(milliseconds: 200)), '00:01');
  });

  test('BulkOp matches the server rules', () {
    final countdown = TimerDef(id: 'c', name: 'c', kind: const Countdown(Duration(seconds: 10)));
    final scheduled = TimerDef(id: 's', name: 's', kind: const AtTime((hour: 9, minute: 0), RepeatDaily()));
    TimerRun run(RunState state, {int remainingMs = 10000}) =>
        TimerRun(runId: 'r', timerId: 'c', state: state, endsAt: null, remainingMs: remainingMs, totalMs: 10000);

    expect(BulkOp.start.appliesTo(countdown, run(RunState.idle)), isTrue);
    expect(BulkOp.start.appliesTo(countdown, run(RunState.paused)), isTrue);
    expect(BulkOp.start.appliesTo(countdown, run(RunState.running)), isFalse);

    expect(BulkOp.pause.appliesTo(countdown, run(RunState.running)), isTrue);
    expect(BulkOp.pause.appliesTo(scheduled, run(RunState.running)), isFalse, reason: 'scheduled cannot pause');

    expect(BulkOp.reset.appliesTo(countdown, run(RunState.idle)), isFalse, reason: 'already at start');
    expect(BulkOp.reset.appliesTo(countdown, run(RunState.idle, remainingMs: 70000)), isTrue, reason: '+1m added');
    expect(BulkOp.reset.appliesTo(countdown, run(RunState.firing, remainingMs: 0)), isTrue);
    expect(BulkOp.reset.appliesTo(scheduled, run(RunState.running)), isTrue);
  });
}
