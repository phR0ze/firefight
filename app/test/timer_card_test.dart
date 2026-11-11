import 'package:firefight/models.dart';
import 'package:firefight/theme.dart';
import 'package:firefight/widgets/timer_card.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  final now = DateTime.utc(2026, 11, 9, 12);
  final def = TimerDef(
    id: 't',
    name: 'Tea steep',
    kind: const Countdown(Duration(minutes: 4)),
    actions: [TimerAction(kind: const SoundAction(sound: 'chime', durationSecs: null))],
  );

  TimerRun run(RunState state, {Duration? left}) => TimerRun(
    runId: 'r',
    timerId: 't',
    state: state,
    endsAt: state == RunState.running ? now.add(left!) : null,
    remainingMs: state == RunState.firing ? 0 : (left ?? const Duration(minutes: 4)).inMilliseconds,
    totalMs: 240000,
  );

  Future<void> pump(WidgetTester tester, TimerRun r) => tester.pumpWidget(
    MaterialApp(
      theme: buildTheme(),
      home: Scaffold(
        body: SizedBox(
          width: 380,
          child: TimerCard(def: def, run: r, serverNow: () => now),
        ),
      ),
    ),
  );

  testWidgets('running card shows remaining time and pill', (tester) async {
    await pump(tester, run(RunState.running, left: const Duration(minutes: 3, seconds: 12)));
    expect(find.text('Tea steep'), findsOneWidget);
    expect(find.text('Countdown · 04:00'), findsOneWidget);
    expect(find.text('RUNNING'), findsOneWidget);
    expect(find.text('03:12'), findsOneWidget);
    expect(find.text('chime'), findsOneWidget);
  });

  testWidgets('firing card is highlighted', (tester) async {
    await pump(tester, run(RunState.firing));
    expect(find.text('FIRING'), findsOneWidget);
    expect(find.text('00:00'), findsOneWidget);
    final box = tester.widget<Container>(find.byType(Container).first);
    final border = (box.decoration! as BoxDecoration).border! as Border;
    expect(border.top.color, Palette.amber);
  });

  testWidgets('idle card shows full duration', (tester) async {
    await pump(tester, run(RunState.idle));
    expect(find.text('IDLE'), findsOneWidget);
    expect(find.text('04:00'), findsOneWidget);
  });
}
