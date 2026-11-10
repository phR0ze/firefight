import 'package:firefight/deck/deck_page.dart';
import 'package:firefight/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets('shows a banner with a settings shortcut when the server is unreachable', (tester) async {
    tester.view.physicalSize = const Size(400, 800);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    var settingsOpened = false;
    // Port 1 on loopback refuses connections.
    await tester.pumpWidget(
      MaterialApp(
        theme: buildTheme(),
        home: DeckPage(server: '127.0.0.1:1', token: 't', onSettings: () => settingsOpened = true),
      ),
    );
    expect(find.textContaining("Can't reach"), findsNothing, reason: 'no banner during the first attempt');

    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 500)));
    await tester.pump();
    expect(find.textContaining("Can't reach the server at 127.0.0.1:1"), findsOneWidget);
    expect(tester.takeException(), isNull);

    await tester.tap(find.text('Settings'));
    expect(settingsOpened, isTrue);

    // Dispose the deck so its reconnect timer is cancelled.
    await tester.pumpWidget(const SizedBox());
  });
}
