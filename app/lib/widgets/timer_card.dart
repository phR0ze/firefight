import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter/scheduler.dart';

import '../models.dart';
import '../theme.dart';

/// Rebuilds [builder] every frame while [active] so countdowns animate smoothly.
class Ticking extends StatefulWidget {
  const Ticking({super.key, required this.builder, this.active = true});
  final WidgetBuilder builder;
  final bool active;

  @override
  State<Ticking> createState() => _TickingState();
}

class _TickingState extends State<Ticking> with SingleTickerProviderStateMixin {
  late final Ticker _ticker = createTicker((_) => setState(() {}));

  @override
  void initState() {
    super.initState();
    if (widget.active) _ticker.start();
  }

  @override
  void didUpdateWidget(Ticking old) {
    super.didUpdateWidget(old);
    if (widget.active && !_ticker.isActive) {
      _ticker.start();
    } else if (!widget.active && _ticker.isActive) {
      _ticker.stop();
    }
  }

  @override
  void dispose() {
    _ticker.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => widget.builder(context);
}

/// Circular progress ring with the remaining time in the middle.
class TimerRing extends StatelessWidget {
  const TimerRing({super.key, required this.fraction, required this.color, required this.label, this.size = 96});
  final double fraction;
  final Color color;
  final String label;
  final double size;

  @override
  Widget build(BuildContext context) {
    final stroke = size * 0.07;
    return SizedBox.square(
      dimension: size,
      child: CustomPaint(
        painter: _RingPainter(fraction: fraction, color: color, stroke: stroke),
        child: Center(
          child: Padding(
            padding: EdgeInsets.all(stroke * 2),
            child: FittedBox(
              child: Text(
                label,
                style: TextStyle(fontFamily: monoFamily, fontWeight: FontWeight.w700, fontSize: size * 0.22),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _RingPainter extends CustomPainter {
  _RingPainter({required this.fraction, required this.color, required this.stroke});
  final double fraction;
  final Color color;
  final double stroke;

  @override
  void paint(Canvas canvas, Size size) {
    final rect = Offset.zero & size;
    final arc = rect.deflate(stroke / 2);
    final paint = Paint()
      ..style = PaintingStyle.stroke
      ..strokeWidth = stroke
      ..strokeCap = StrokeCap.round;
    canvas.drawArc(arc, 0, 2 * math.pi, false, paint..color = Palette.track);
    if (fraction > 0) {
      canvas.drawArc(arc, -math.pi / 2, 2 * math.pi * fraction, false, paint..color = color);
    }
  }

  @override
  bool shouldRepaint(_RingPainter old) => old.fraction != fraction || old.color != color || old.stroke != stroke;
}

class StatusPill extends StatelessWidget {
  const StatusPill(this.state, {super.key});
  final RunState state;

  @override
  Widget build(BuildContext context) {
    final c = pillColors(state);
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
      decoration: BoxDecoration(color: c.bg, borderRadius: BorderRadius.circular(6)),
      child: Text(
        state.name.toUpperCase(),
        style: TextStyle(color: c.fg, fontSize: 11, fontWeight: FontWeight.w700, letterSpacing: 1),
      ),
    );
  }
}

/// One line of the "ON FINISH" list, e.g. "♪ Gong · loop".
class ActionLine extends StatelessWidget {
  const ActionLine(this.action, {super.key, this.canLoop = true});
  final TimerAction action;
  final bool canLoop;

  @override
  Widget build(BuildContext context) {
    final (icon, text) = switch (action.kind) {
      SoundAction(:final sound, :final loopUntilDismissed) => (
        '♪',
        '${sound.split('/').last}${canLoop && loopUntilDismissed ? ' · loop' : ''}',
      ),
      CommandAction(:final cmd) => (r'$', cmd),
    };
    return Padding(
      padding: const EdgeInsets.only(top: 6),
      child: Row(
        children: [
          SizedBox(
            width: 18,
            child: Text(icon, style: const TextStyle(color: Palette.amber, fontSize: 12)),
          ),
          Expanded(
            child: Text(text, maxLines: 1, overflow: TextOverflow.ellipsis, style: const TextStyle(fontSize: 13)),
          ),
        ],
      ),
    );
  }
}

/// The timer card from the control deck mockup. [footer] holds the buttons.
class TimerCard extends StatelessWidget {
  const TimerCard({
    super.key,
    required this.def,
    required this.run,
    required this.serverNow,
    this.footer,
    this.trailing,
    this.ringSize = 96,
  });

  final TimerDef def;
  final TimerRun? run;
  final DateTime Function() serverNow;
  final Widget? footer;
  final Widget? trailing;
  final double ringSize;

  @override
  Widget build(BuildContext context) {
    final state = run?.state ?? RunState.idle;
    final firing = state == RunState.firing;
    return Container(
      padding: const EdgeInsets.all(20),
      decoration: BoxDecoration(
        color: Palette.card,
        borderRadius: BorderRadius.circular(16),
        border: Border.all(color: firing ? Palette.amber : Palette.border),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        mainAxisSize: MainAxisSize.min,
        children: [
          Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      def.name,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: const TextStyle(fontSize: 17, fontWeight: FontWeight.w600),
                    ),
                    const SizedBox(height: 2),
                    Text(def.subtitle, style: const TextStyle(color: Palette.muted, fontSize: 12)),
                  ],
                ),
              ),
              const SizedBox(width: 8),
              StatusPill(state),
              ?trailing,
            ],
          ),
          const SizedBox(height: 16),
          Row(
            children: [
              Ticking(
                active: state == RunState.running,
                builder: (_) {
                  final now = serverNow();
                  final r = run;
                  final remaining = r?.remainingAt(now) ?? Duration.zero;
                  return TimerRing(
                    size: ringSize,
                    fraction: r?.fractionAt(now) ?? 0,
                    color: ringColor(state),
                    label: switch (def.kind) {
                      AtTime(:final time) when state == RunState.idle => formatHm(time),
                      _ => formatRemaining(remaining),
                    },
                  );
                },
              ),
              const SizedBox(width: 20),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    const Text('ON FINISH', style: TextStyle(color: Palette.muted, fontSize: 11, letterSpacing: 1)),
                    if (def.actions.isEmpty)
                      const Padding(
                        padding: EdgeInsets.only(top: 6),
                        child: Text('nothing', style: TextStyle(color: Palette.muted, fontSize: 13)),
                      ),
                    for (final a in def.actions.take(3)) ActionLine(a, canLoop: def.allowClientDismiss),
                    if (def.actions.length > 3)
                      Padding(
                        padding: const EdgeInsets.only(top: 6),
                        child: Text(
                          '+${def.actions.length - 3} more',
                          style: const TextStyle(color: Palette.muted, fontSize: 12),
                        ),
                      ),
                  ],
                ),
              ),
            ],
          ),
          if (footer != null) ...[const SizedBox(height: 18), footer!],
        ],
      ),
    );
  }
}
