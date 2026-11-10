import 'package:firefight/main.dart';
import 'package:firefight/settings.dart';
import 'package:firefight/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  Future<void> pump(WidgetTester tester, {required bool deckOnly, required ValueChanged<AppSettings> onSave}) =>
      tester.pumpWidget(
        MaterialApp(
          theme: buildTheme(),
          home: SetupPage(
            initial: AppSettings(server: ''),
            deckOnly: deckOnly,
            onSave: onSave,
          ),
        ),
      );

  testWidgets('deck-only setup hides the mode choice and saves deck mode', (tester) async {
    AppSettings? saved;
    await pump(tester, deckOnly: true, onSave: (s) => saved = s);
    expect(find.text('Client display'), findsNothing);
    expect(find.text('Control deck'), findsNothing);

    await tester.enterText(find.widgetWithText(TextField, 'Server (host:port)'), 'deck-host:7878');
    await tester.enterText(find.widgetWithText(TextField, 'Admin token'), 'secret');
    await tester.tap(find.text('Continue'));
    await tester.pump();
    expect(saved?.mode, AppMode.server);
    expect(saved?.server, 'deck-host:7878');
    expect(saved?.adminToken, 'secret');
  });

  testWidgets('desktop setup offers both modes', (tester) async {
    await pump(tester, deckOnly: false, onSave: (_) {});
    expect(find.text('Control deck'), findsOneWidget);
    expect(find.text('Client display'), findsOneWidget);
  });
}
