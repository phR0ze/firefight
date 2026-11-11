import 'package:firefight/api/stores.dart';
import 'package:firefight/client/client_page.dart';
import 'package:firefight/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets('has no settings: the agent runs the display', (tester) async {
    // Port 1 on loopback refuses connections.
    await tester.pumpWidget(MaterialApp(theme: buildTheme(), home: const ClientPage(agentPort: 1)));
    expect(find.byTooltip('Settings'), findsNothing);
    await tester.pumpWidget(const SizedBox());
  });

  test('only newly firing runs of focus-on-fire timers bring the display forward', () {
    Map<String, dynamic> timer(String id, {required bool focus}) => {
      'id': id,
      'name': id,
      'kind': {'type': 'countdown', 'duration_secs': 60},
      'focus_on_fire': focus,
    };
    Map<String, dynamic> run(String timerId, String state) => {
      'run_id': 'run-$timerId',
      'timer_id': timerId,
      'state': state,
      'ends_at': null,
      'remaining_ms': 0,
      'total_ms': 60000,
    };
    AgentStore storeWith(List<Map<String, dynamic>> runs) => AgentStore(port: 1)
      ..onEvent({
        'type': 'state',
        'status': 'approved',
        'client_id': 'c',
        'name': 'n',
        'timers': [timer('loud', focus: true), timer('quiet', focus: false)],
        'runs': runs,
        'server_now': DateTime.utc(2026).toIso8601String(),
      });

    expect(firingNeedsFocus(storeWith([run('quiet', 'firing')]), {}), isFalse);
    expect(firingNeedsFocus(storeWith([run('loud', 'running')]), {}), isFalse);
    expect(firingNeedsFocus(storeWith([run('loud', 'firing')]), {}), isTrue);
    expect(firingNeedsFocus(storeWith([run('loud', 'firing')]), {'run-loud'}), isFalse, reason: 'alerts once');
  });
}
