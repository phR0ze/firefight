import 'package:firefight/deck/timer_editor.dart';
import 'package:firefight/models.dart';
import 'package:firefight/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  Future<Future<TimerDef?> Function()> open(
    WidgetTester tester, {
    TimerDef? initial,
    Size size = const Size(1280, 820),
  }) async {
    TimerDef? result;
    var closed = false;
    tester.view.physicalSize = size;
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(
      MaterialApp(
        theme: buildTheme(),
        home: Builder(
          builder: (context) => Scaffold(
            body: Center(
              child: FilledButton(
                onPressed: () async {
                  result = await showDialog<TimerDef>(
                    context: context,
                    builder: (_) => TimerEditor(initial: initial, clients: const [], onTestRun: (_, _) async {}),
                  );
                  closed = true;
                },
                child: const Text('open'),
              ),
            ),
          ),
        ),
      ),
    );
    return () async {
      expect(closed, isTrue, reason: 'dialog should have closed');
      return result;
    };
  }

  testWidgets('creates a countdown with sound then command', (tester) async {
    final result = await open(tester);
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();

    expect(find.text('1 · WHEN'), findsOneWidget);
    expect(find.text('2 · THEN DO'), findsOneWidget);
    expect(tester.takeException(), isNull);

    await tester.enterText(find.byType(TextField).first, 'Lab closes');
    await tester.tap(find.text('25m'));
    await tester.tap(find.text('+ Sound'));
    await tester.pump();
    await tester.tap(find.text('+ Command'));
    await tester.pump();
    await tester.enterText(find.widgetWithText(TextFormField, '').last, 'loginctl lock-session');
    await tester.tap(find.text('Save timer'));
    await tester.pumpAndSettle();

    final def = (await result())!;
    expect(def.name, 'Lab closes');
    expect((def.kind as Countdown).duration, const Duration(minutes: 25));
    expect(def.actions.map((a) => a.kind.runtimeType), [SoundAction, CommandAction]);
    expect((def.actions[1].kind as CommandAction).cmd, 'loginctl lock-session');
    expect(def.actions[1].mode, ActionMode.afterPrevious);
  });

  testWidgets('validates empty command and empty name', (tester) async {
    await open(tester);
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('+ Command'));
    await tester.pump();
    await tester.tap(find.text('Save timer'));
    await tester.pump();
    expect(find.text('Give the timer a name.'), findsOneWidget);
    expect(find.text('Save timer'), findsOneWidget, reason: 'dialog stays open');
  });

  testWidgets('edits an at-time timer with specific days', (tester) async {
    final initial = TimerDef(
      id: 'x',
      name: 'Shutdown',
      kind: const AtTime((hour: 22, minute: 30), RepeatDays({Weekday.fri})),
    );
    final result = await open(tester, initial: initial);
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();
    expect(find.text('Edit timer'), findsOneWidget);
    expect(find.text('22'), findsOneWidget);
    expect(find.text('30'), findsOneWidget);
    await tester.tap(find.text('Mon'));
    await tester.tap(find.text('Clients can dismiss'));
    await tester.pump();
    await tester.tap(find.text('Save timer'));
    await tester.pumpAndSettle();
    final def = (await result())!;
    expect(def.id, 'x');
    expect(def.allowClientDismiss, isFalse);
    expect(((def.kind as AtTime).repeat as RepeatDays).days, {Weekday.mon, Weekday.fri});
  });

  testWidgets('phone layout uses tabs and saves from the app bar', (tester) async {
    final result = await open(tester, size: const Size(400, 800));
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull, reason: 'no overflow at phone width');
    expect(find.text('1 · When'), findsOneWidget);
    expect(find.text('2 · Then do'), findsOneWidget);
    expect(find.text('Save timer'), findsNothing, reason: 'save lives in the app bar');

    // Saving without a name shows the error above the tabs.
    await tester.tap(find.text('Save'));
    await tester.pump();
    expect(find.text('Give the timer a name.'), findsOneWidget);

    await tester.enterText(find.byType(TextField).first, 'Phone timer');
    await tester.tap(find.text('2 · Then do'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('+ Sound'));
    await tester.pump();
    await tester.tap(find.text('+ Command'));
    await tester.pump();
    await tester.enterText(find.widgetWithText(TextFormField, '').last, 'true');
    expect(tester.takeException(), isNull, reason: 'two steps fit at phone width');
    await tester.tap(find.text('Save'));
    await tester.pumpAndSettle();

    final def = (await result())!;
    expect(def.name, 'Phone timer');
    expect(def.actions.map((a) => a.kind.runtimeType), [SoundAction, CommandAction]);
  });

  testWidgets('action steps lay out without overflow in a narrow dialog', (tester) async {
    await open(tester, size: const Size(900, 820));
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('+ Sound'));
    await tester.pump();
    await tester.tap(find.text('+ Command'));
    await tester.pump();
    expect(tester.takeException(), isNull);
  });
}
