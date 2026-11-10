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
}
