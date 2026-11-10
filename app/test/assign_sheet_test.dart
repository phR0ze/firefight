import 'package:firefight/api/admin_api.dart';
import 'package:firefight/api/stores.dart';
import 'package:firefight/deck/client_card.dart';
import 'package:firefight/models.dart';
import 'package:firefight/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets('assign sheet toggles timers and returns them in deck order', (tester) async {
    tester.view.physicalSize = const Size(400, 800);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final timers = [
      TimerDef(id: 'a', name: 'Alpha', kind: const Countdown(Duration(minutes: 1))),
      TimerDef(id: 'b', name: 'Bravo', kind: const Countdown(Duration(minutes: 2))),
      TimerDef(id: 'c', name: 'Charlie', kind: const Countdown(Duration(minutes: 3))),
    ];
    const client = ClientView(
      id: 'x',
      name: 'Test box',
      hostname: 'host',
      status: ClientStatus.approved,
      online: true,
      assignedTimers: ['c'],
    );
    List<String>? result;
    await tester.pumpWidget(
      MaterialApp(
        theme: buildTheme(),
        home: Builder(
          builder: (context) => Scaffold(
            body: FilledButton(
              onPressed: () async => result = await showModalBottomSheet<List<String>>(
                context: context,
                isScrollControlled: true,
                builder: (_) => AssignSheet(client: client, timers: timers),
              ),
              child: const Text('open'),
            ),
          ),
        ),
      ),
    );
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();
    expect(find.text('Timers for Test box'), findsOneWidget);

    await tester.tap(find.text('Alpha'));
    await tester.tap(find.text('Charlie'));
    await tester.tap(find.text('Bravo'));
    await tester.pump();
    await tester.tap(find.text('Save'));
    await tester.pumpAndSettle();
    expect(result, ['a', 'b']);
  });

  testWidgets('clients panel fits a phone and offers Assign timers', (tester) async {
    tester.view.physicalSize = const Size(360, 700);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final store = DeckStore(server: 'unused:1', token: 't');
    addTearDown(store.dispose);
    store.timers['a'] = TimerDef(id: 'a', name: 'Alpha', kind: const Countdown(Duration(minutes: 1)));
    store.clients['x'] = const ClientView(
      id: 'x',
      name: 'A client with a fairly long display name',
      hostname: 'host',
      status: ClientStatus.approved,
      online: true,
      assignedTimers: ['a'],
    );
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
    expect(tester.takeException(), isNull);
    await tester.tap(find.byTooltip('More'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Assign timers…'));
    await tester.pumpAndSettle();
    expect(find.text('Timers for A client with a fairly long display name'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });
}
