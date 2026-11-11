import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:web_socket_channel/io.dart';
import 'package:web_socket_channel/web_socket_channel.dart';

import '../models.dart';

/// Tracks the offset between a remote clock and ours from `server_now` samples.
class ClockSync {
  Duration _offset = Duration.zero;
  bool _seeded = false;

  void observe(String? serverNow) {
    if (serverNow == null) return;
    final sample = DateTime.parse(serverNow).difference(DateTime.now());
    // Exponential smoothing keeps one slow message from jerking the display.
    _offset = _seeded ? _offset * 0.8 + sample * 0.2 : sample;
    _seeded = true;
  }

  DateTime now() => DateTime.now().toUtc().add(_offset);
}

/// A WebSocket that reconnects on failure and hands decoded JSON objects to [onEvent].
abstract class _LiveSocket extends ChangeNotifier {
  WebSocketChannel? _channel;
  StreamSubscription<dynamic>? _sub;
  Timer? _retry;
  int _quickRetries = 0;
  bool _disposed = false;
  bool connected = false;
  String? error;
  final clock = ClockSync();

  Uri get uri;
  Map<String, dynamic>? get headers => null;
  void onEvent(Map<String, dynamic> ev);
  void onDisconnected() {}

  void connect() {
    _retry?.cancel();
    final channel = IOWebSocketChannel.connect(uri, headers: headers, pingInterval: const Duration(seconds: 15));
    _channel = channel;
    channel.ready.then((_) {
      if (_disposed || _channel != channel) return;
      connected = true;
      error = null;
      _quickRetries = 0;
      notifyListeners();
    }, onError: (Object e) => _closed(channel, e));
    _sub = channel.stream.listen(
      (data) {
        final ev = jsonDecode(data as String) as Map<String, dynamic>;
        clock.observe(ev['server_now'] as String?);
        onEvent(ev);
        notifyListeners();
      },
      onError: (Object e) => _closed(channel, e),
      onDone: () => _closed(channel, channel.closeReason ?? 'connection closed'),
      cancelOnError: true,
    );
  }

  /// Drop the current socket, which may have silently died while the app was
  /// suspended, and connect again right away.
  ///
  /// The network can take a moment to come back after a resume, so the next
  /// few failures retry quickly without surfacing an error.
  void reconnect() {
    if (_disposed) return;
    _retry?.cancel();
    _sub?.cancel();
    _channel?.sink.close();
    _channel = null;
    connected = false;
    error = null;
    _quickRetries = 6;
    notifyListeners();
    connect();
  }

  void _closed(WebSocketChannel channel, Object reason) {
    // Ignore late errors from a socket [reconnect] already replaced.
    if (_disposed || _channel != channel) return;
    _sub?.cancel();
    final wasConnected = connected;
    connected = false;
    _retry?.cancel();
    if (_quickRetries > 0) {
      _quickRetries--;
      _retry = Timer(const Duration(milliseconds: 500), connect);
    } else {
      error = _describe(reason);
      _retry = Timer(Duration(seconds: wasConnected ? 1 : 3), connect);
    }
    onDisconnected();
    notifyListeners();
  }

  String _describe(Object e) {
    final s = e.toString();
    if (s.contains('401')) return 'invalid admin token';
    if (s.contains('Connection refused')) return 'connection refused';
    return s.length > 120 ? '${s.substring(0, 120)}…' : s;
  }

  void send(Map<String, dynamic> msg) => _channel?.sink.add(jsonEncode(msg));

  @override
  void dispose() {
    _disposed = true;
    _retry?.cancel();
    _sub?.cancel();
    _channel?.sink.close();
    super.dispose();
  }
}

/// Live state of the server, for the control deck.
class DeckStore extends _LiveSocket {
  DeckStore({required this.server, required this.token});

  final String server;
  final String token;

  /// Insertion-ordered so cards keep their position.
  final timers = <String, TimerDef>{};

  /// Shared run of each timer, followed by every client without its own copy.
  final runs = <String, TimerRun>{};

  /// Copies of a timer's run that a client follows on its own: client id -> timer id -> run.
  final clientRuns = <String, Map<String, TimerRun>>{};
  final clients = <String, ClientView>{};

  /// The run [clientId] follows for [timerId].
  TimerRun? runFor(String clientId, String timerId) => clientRuns[clientId]?[timerId] ?? runs[timerId];

  /// How many clients follow their own copy of [timerId]'s run.
  int separateCount(String timerId) => clientRuns.values.where((m) => m.containsKey(timerId)).length;

  @override
  Uri get uri => Uri.parse('ws://$server/api/admin/ws');

  @override
  Map<String, dynamic> get headers => {'Authorization': 'Bearer $token'};

  @override
  void onEvent(Map<String, dynamic> ev) {
    switch (ev['type']) {
      case 'snapshot':
        timers
          ..clear()
          ..addEntries([
            for (final t in ev['timers'] as List)
              MapEntry((t as Map<String, dynamic>)['id'] as String, TimerDef.fromJson(t)),
          ]);
        runs
          ..clear()
          ..addEntries([
            for (final r in ev['runs'] as List)
              MapEntry((r as Map<String, dynamic>)['timer_id'] as String, TimerRun.fromJson(r)),
          ]);
        clientRuns.clear();
        // Absent from servers older than per-client runs.
        for (final cr in ev['client_runs'] as List? ?? const []) {
          _setClientRun(cr as Map<String, dynamic>);
        }
        clients
          ..clear()
          ..addEntries([
            for (final c in ev['clients'] as List)
              MapEntry((c as Map<String, dynamic>)['id'] as String, ClientView.fromJson(c)),
          ]);
      case 'timer_updated':
        final t = TimerDef.fromJson(ev['timer'] as Map<String, dynamic>);
        timers[t.id] = t;
        runs[t.id] = TimerRun.fromJson(ev['run'] as Map<String, dynamic>);
      case 'timer_deleted':
        timers.remove(ev['id']);
        runs.remove(ev['id']);
      case 'run_update':
        final r = TimerRun.fromJson(ev['run'] as Map<String, dynamic>);
        runs[r.timerId] = r;
      case 'client_run_update':
        _setClientRun(ev);
      case 'client_run_removed':
        final id = ev['client_id'] as String;
        clientRuns[id]?.remove(ev['timer_id']);
        if (clientRuns[id]?.isEmpty ?? false) clientRuns.remove(id);
      case 'client_updated':
        final c = ClientView.fromJson(ev['client'] as Map<String, dynamic>);
        clients[c.id] = c;
      case 'client_deleted':
        clients.remove(ev['id']);
        clientRuns.remove(ev['id']);
    }
  }

  void _setClientRun(Map<String, dynamic> j) {
    final r = TimerRun.fromJson(j['run'] as Map<String, dynamic>);
    (clientRuns[j['client_id'] as String] ??= {})[r.timerId] = r;
  }
}

enum AgentStatus { disconnected, pending, approved }

/// State from the local agent, for the client display.
class AgentStore extends _LiveSocket {
  AgentStore({required this.port});

  final int port;
  AgentStatus status = AgentStatus.disconnected;
  String? clientId;
  String name = '';
  List<TimerDef> timers = [];
  Map<String, TimerRun> runs = {};

  @override
  Uri get uri => Uri.parse('ws://127.0.0.1:$port');

  @override
  void onEvent(Map<String, dynamic> ev) {
    if (ev['type'] != 'state') return;
    status = AgentStatus.values.byName(ev['status'] as String);
    clientId = ev['client_id'] as String?;
    name = ev['name'] as String;
    timers = [for (final t in ev['timers'] as List) TimerDef.fromJson(t as Map<String, dynamic>)];
    runs = {
      for (final r in ev['runs'] as List) (r as Map<String, dynamic>)['timer_id'] as String: TimerRun.fromJson(r),
    };
  }

  void dismiss(String timerId) => send({'type': 'dismiss', 'timer_id': timerId});
}
