import 'dart:convert';

import 'package:http/http.dart' as http;

import '../models.dart';

class ApiException implements Exception {
  ApiException(this.status, this.message);
  final int status;
  final String message;
  @override
  String toString() => message;
}

/// REST client for the server's `/api/admin` endpoints.
class AdminApi {
  AdminApi({required this.server, required this.token, http.Client? client}) : _http = client ?? http.Client();

  final String server;
  final String token;
  final http.Client _http;

  Uri _uri(String path, [Map<String, String>? query]) =>
      Uri.parse('http://$server/api/admin$path').replace(queryParameters: query);

  Map<String, String> get _headers => {'Authorization': 'Bearer $token', 'Content-Type': 'application/json'};

  Future<dynamic> _send(String method, String path, {Object? body, Map<String, String>? query}) async {
    final req = http.Request(method, _uri(path, query))..headers.addAll(_headers);
    if (body != null) req.body = jsonEncode(body);
    final res = await http.Response.fromStream(await _http.send(req));
    if (res.statusCode >= 400) {
      var msg = 'HTTP ${res.statusCode}';
      try {
        msg = (jsonDecode(res.body) as Map<String, dynamic>)['error'] as String? ?? msg;
      } on Object {
        // keep the status text
      }
      throw ApiException(res.statusCode, msg);
    }
    return res.body.isEmpty ? null : jsonDecode(res.body);
  }

  Future<TimerDef> createTimer(TimerDef def) async =>
      TimerDef.fromJson(await _send('POST', '/timers', body: def.toDraftJson()) as Map<String, dynamic>);

  Future<TimerDef> updateTimer(TimerDef def) async =>
      TimerDef.fromJson(await _send('PUT', '/timers/${def.id}', body: def.toDraftJson()) as Map<String, dynamic>);

  Future<void> deleteTimer(String id) => _send('DELETE', '/timers/$id');

  /// `op` is one of start, pause, reset, dismiss, add.
  Future<void> control(String id, String op, {int? addSecs}) =>
      _send('POST', '/timers/$id/$op', query: addSecs == null ? null : {'secs': '$addSecs'});

  Future<void> approveClient(String id) => _send('POST', '/clients/$id/approve');

  Future<void> removeClient(String id) => _send('DELETE', '/clients/$id');

  Future<void> renameClient(String id, String name) => _send('PATCH', '/clients/$id', body: {'name': name});

  Future<void> setAssignments(String id, List<String> timers) =>
      _send('PUT', '/clients/$id/assignments', body: {'timers': timers});

  /// Apply [op] to every timer assigned to a client.
  Future<void> bulkClient(String id, BulkOp op) => _send('POST', '/clients/$id/${op.name}');

  /// `op` is dismiss or add, applied to one of a client's timers for that client only.
  Future<void> controlClient(String clientId, String timerId, String op, {int? addSecs}) =>
      _send('POST', '/clients/$clientId/timers/$timerId/$op', query: addSecs == null ? null : {'secs': '$addSecs'});

  /// Apply [op] to every timer assigned to any approved client.
  Future<void> bulkAll(BulkOp op) => _send('POST', '/clients/${op.name}');

  Future<void> testRun(String clientId, List<TimerAction> actions) => _send(
    'POST',
    '/clients/$clientId/test',
    body: {
      'actions': [for (final a in actions) a.toJson()],
    },
  );
}
