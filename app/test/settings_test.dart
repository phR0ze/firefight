import 'dart:io';

import 'package:firefight/settings.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('--token-file reads and trims the admin token', () {
    final dir = Directory.systemTemp.createTempSync('firefight-settings');
    addTearDown(() => dir.deleteSync(recursive: true));
    final file = File('${dir.path}/token')..writeAsStringSync('  secret-token\n');

    final s = AppSettings()..applyArgs(['--mode=server', '--server=deck-host:7878', '--token-file=${file.path}']);
    expect(s.mode, AppMode.server);
    expect(s.server, 'deck-host:7878');
    expect(s.adminToken, 'secret-token');
  });

  test('a missing or empty --token-file keeps the existing token', () {
    final s = AppSettings(adminToken: 'saved')..applyArgs(['--token-file=/nonexistent/firefight-token']);
    expect(s.adminToken, 'saved');
  });

  test('flags without a value are ignored', () {
    final s = AppSettings(adminToken: 'saved')..applyArgs(['--start-hidden', '--no-tray']);
    expect(s.adminToken, 'saved');
  });
}
