import 'package:firefight/api/stores.dart';
import 'package:firefight/models.dart';
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
