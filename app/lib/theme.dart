import 'package:flutter/material.dart';

import 'models.dart';

/// Palette taken from docs/design mockups.
abstract final class Palette {
  static const bg = Color(0xFF0F1115);
  static const panel = Color(0xFF151820);
  static const card = Color(0xFF171A21);
  static const field = Color(0xFF111318);
  static const border = Color(0xFF262A33);
  static const track = Color(0xFF262B35);
  static const text = Color(0xFFE6E8EB);
  static const muted = Color(0xFF8B93A1);
  static const amber = Color(0xFFFFB224);
  static const blue = Color(0xFF6CA6FF);
  static const slate = Color(0xFF4A5263);
  static const green = Color(0xFF4CC38A);
  static const red = Color(0xFFFF6B6B);
}

const monoFamily = 'monospace';

ThemeData buildTheme() {
  final base = ThemeData(
    brightness: Brightness.dark,
    colorScheme: const ColorScheme.dark(
      primary: Palette.amber,
      onPrimary: Color(0xFF1A1300),
      secondary: Palette.blue,
      surface: Palette.card,
      onSurface: Palette.text,
      error: Palette.red,
    ),
    scaffoldBackgroundColor: Palette.bg,
    dividerColor: Palette.border,
    dividerTheme: const DividerThemeData(color: Palette.border, space: 1, thickness: 1),
    useMaterial3: true,
  );
  final shape = RoundedRectangleBorder(borderRadius: BorderRadius.circular(10));
  return base.copyWith(
    textTheme: base.textTheme.apply(bodyColor: Palette.text, displayColor: Palette.text),
    inputDecorationTheme: InputDecorationTheme(
      filled: true,
      fillColor: Palette.field,
      isDense: true,
      contentPadding: const EdgeInsets.symmetric(horizontal: 12, vertical: 14),
      border: OutlineInputBorder(
        borderRadius: BorderRadius.circular(10),
        borderSide: const BorderSide(color: Palette.border),
      ),
      enabledBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(10),
        borderSide: const BorderSide(color: Palette.border),
      ),
      focusedBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(10),
        borderSide: const BorderSide(color: Palette.amber),
      ),
      labelStyle: const TextStyle(color: Palette.muted),
    ),
    filledButtonTheme: FilledButtonThemeData(
      style: FilledButton.styleFrom(
        shape: shape,
        minimumSize: const Size(0, 44),
        textStyle: const TextStyle(fontWeight: FontWeight.w600),
      ),
    ),
    outlinedButtonTheme: OutlinedButtonThemeData(
      style: OutlinedButton.styleFrom(
        shape: shape,
        minimumSize: const Size(44, 44),
        foregroundColor: Palette.text,
        side: const BorderSide(color: Palette.border),
        padding: const EdgeInsets.symmetric(horizontal: 14),
      ),
    ),
    dialogTheme: const DialogThemeData(backgroundColor: Palette.panel),
    tooltipTheme: const TooltipThemeData(waitDuration: Duration(milliseconds: 500)),
  );
}

/// Neutral filled button used for Start/Pause (the mockup's dark buttons).
ButtonStyle neutralButton() =>
    FilledButton.styleFrom(backgroundColor: const Color(0xFF262A33), foregroundColor: Palette.text);

({Color fg, Color bg}) pillColors(RunState s) => switch (s) {
  RunState.firing => (fg: const Color(0xFF1A1300), bg: Palette.amber),
  RunState.running => (fg: const Color(0xFF8FB4FF), bg: const Color(0xFF1E2A44)),
  RunState.paused || RunState.idle => (fg: const Color(0xFF9AA3B2), bg: const Color(0xFF2A2E37)),
};

Color ringColor(RunState s) => switch (s) {
  RunState.firing => Palette.amber,
  RunState.running => Palette.blue,
  RunState.paused => Palette.slate,
  RunState.idle => Palette.track,
};

/// Widths below this use the phone layout (tabs, full-screen editor).
const compactBreakpoint = 700.0;
