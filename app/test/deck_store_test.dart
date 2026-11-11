import 'package:firefight/api/admin_api.dart';
import 'package:firefight/api/stores.dart';
import 'package:firefight/deck/client_card.dart';
import 'package:firefight/models.dart';
import 'package:firefight/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

Map<String, dynamic> run(String timerId, String state, {String runId = 'r'}) => {
  'run_id': runId,
  'timer_id': timerId,
  'state': state,
  'ends_at': null,
  'remaining_ms': 60000,
  'total_ms': 60000,
};

void main() {
  test('clients follow their own run when they have one, else the shared run', () {
    final store = DeckStore(server: 'unused:1', token: 't');
    addTearDown(store.dispose);
    store.onEvent({
      'type': 'snapshot',
      'timers': <dynamic>[],
      'runs': [run('t', 'idle')],
      'client_runs': [
        {'client_id': 'a', 'run': run('t', 'paused', runId: 'ra')},
      ],
      'clients': <dynamic>[],
    });
    expect(store.runFor('a', 't')?.state, RunState.paused);
    expect(store.runFor('b', 't')?.state, RunState.idle);
    expect(store.separateCount('t'), 1);

    store.onEvent({'type': 'client_run_update', 'client_id': 'b', 'run': run('t', 'running', runId: 'rb')});
    expect(store.runFor('b', 't')?.state, RunState.running);
    expect(store.separateCount('t'), 2);

    store.onEvent({'type': 'client_run_removed', 'client_id': 'a', 'timer_id': 't'});
    expect(store.runFor('a', 't')?.state, RunState.idle);
    expect(store.clientRuns.containsKey('a'), isFalse);

    store.onEvent({'type': 'client_deleted', 'id': 'b'});
    expect(store.separateCount('t'), 0);
  });

  testWidgets("a client's own running timers can be paused and reset from its card", (tester) async {
    final store = DeckStore(server: 'unused:1', token: 't');
    addTearDown(store.dispose);
    store.onEvent({
      'type': 'snapshot',
      'timers': [TimerDef(id: 't', name: 'Alpha', kind: const Countdown(Duration(minutes: 1))).toJson()],
      'runs': [run('t', 'idle')],
      'client_runs': [
        {
          'client_id': 'a',
          'run': {
            ...run('t', 'running', runId: 'ra'),
            'ends_at': DateTime.now().toUtc().add(const Duration(minutes: 1)).toIso8601String(),
          },
        },
      ],
      'clients': [
        {
          'id': 'a',
          'name': 'A',
          'hostname': 'host',
          'status': 'approved',
          'online': true,
          'last_seen': null,
          'assigned_timers': ['t'],
          'last_result': null,
        },
      ],
    });
    await tester.pumpWidget(
      MaterialApp(
        theme: buildTheme(),
        home: Scaffold(
          body: ClientsPanel(
            store: store,
            api: AdminApi(server: 'unused:1', token: 't'),
          ),
        ),
      ),
    );
    IconButton button(String tooltip) =>
        tester.widget<IconButton>(find.ancestor(of: find.byTooltip(tooltip), matching: find.byType(IconButton)).first);
    expect(button("Start this client's timers").onPressed, isNull);
    expect(button("Pause this client's timers").onPressed, isNotNull);
    expect(button("Reset this client's timers").onPressed, isNotNull);
  });

  testWidgets("a timer chip offers reset and +1 minute for the client's run", (tester) async {
    final store = DeckStore(server: 'unused:1', token: 't');
    addTearDown(store.dispose);
    store.onEvent({
      'type': 'snapshot',
      'timers': [
        TimerDef(id: 'f', name: 'Firing', kind: const Countdown(Duration(minutes: 1))).toJson(),
        TimerDef(id: 'i', name: 'Idle', kind: const Countdown(Duration(minutes: 1))).toJson(),
        TimerDef(id: 's', name: 'Sched', kind: const AtTime((hour: 9, minute: 0), RepeatDaily())).toJson(),
      ],
      'runs': [run('f', 'idle'), run('i', 'idle'), run('s', 'idle')],
      // Only this client's own copy is firing.
      'client_runs': [
        {'client_id': 'a', 'run': run('f', 'firing', runId: 'rf')},
      ],
      'clients': [
        {
          'id': 'a',
          'name': 'A',
          'hostname': 'host',
          'status': 'approved',
          'online': true,
          'last_seen': null,
          'assigned_timers': ['f', 'i', 's'],
          'last_result': null,
        },
      ],
    });
    await tester.pumpWidget(
      MaterialApp(
        theme: buildTheme(),
        home: Scaffold(
          body: ClientsPanel(
            store: store,
            api: AdminApi(server: 'unused:1', token: 't'),
          ),
        ),
      ),
    );
    bool enabled(String text) =>
        tester.widget<PopupMenuItem<String>>(find.widgetWithText(PopupMenuItem<String>, text)).enabled;
    Future<void> open(String chip) async {
      await tester.tap(find.widgetWithText(InputChip, chip));
      await tester.pumpAndSettle();
    }

    Future<void> close() async {
      await tester.tapAt(Offset.zero);
      await tester.pumpAndSettle();
    }

    await open('Firing');
    expect(enabled('Reset'), isTrue);
    expect(enabled('Snooze 1 minute'), isTrue);
    await close();

    await open('Idle');
    expect(enabled('Reset'), isFalse);
    expect(enabled('Add 1 minute'), isTrue);
    await close();

    await open('Sched');
    expect(enabled('Reset'), isFalse);
    expect(enabled('Add 1 minute'), isFalse, reason: 'scheduled timers only snooze while firing');
    await close();
  });

  test('snapshot from a server without per-client runs', () {
    final store = DeckStore(server: 'unused:1', token: 't');
    addTearDown(store.dispose);
    store.onEvent({
      'type': 'snapshot',
      'timers': <dynamic>[],
      'runs': [run('t', 'running')],
      'clients': <dynamic>[],
    });
    expect(store.runFor('a', 't')?.state, RunState.running);
  });
}
