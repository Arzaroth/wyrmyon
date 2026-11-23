import 'dart:math' as math;

import 'package:flutter/material.dart';

abstract final class Palette {
  static const bg = Color(0xFF0B1230);
  static const bg2 = Color(0xFF121C44);
  static const glow = Color(0xFF15275A);
  static const line = Color(0xFF24306A);
  static const fg = Color(0xFFE8F1FF);
  static const dim = Color(0xFF8AA0D0);
  static const cyan = Color(0xFF5FD4FF);
  static const blue = Color(0xFF3B7BFF);
  static const sky = Color(0xFF2FB6FF);
  static const ok = Color(0xFF4ADE80);
  static const tile = Color(0xFF1B2A63);
  static const track = Color(0xFF1B2550);
  static const pillLine = Color(0xFF2B5A7A);
  static const code = Color(0xFF0A1030);
  static const error = Color(0xFFFF7A90);
}

const mono = 'UbuntuMono';

ThemeData wyrmyonTheme() {
  const scheme = ColorScheme.dark(
    primary: Palette.cyan,
    onPrimary: Palette.bg,
    secondary: Palette.blue,
    surface: Palette.bg2,
    onSurface: Palette.fg,
    outline: Palette.line,
    error: Palette.error,
  );
  final base = ThemeData(
    useMaterial3: true,
    colorScheme: scheme,
    fontFamily: 'Ubuntu',
    scaffoldBackgroundColor: Colors.transparent,
  );
  return base.copyWith(
    textTheme: base.textTheme.apply(
      bodyColor: Palette.fg,
      displayColor: Palette.fg,
    ),
    appBarTheme: const AppBarTheme(
      backgroundColor: Colors.transparent,
      surfaceTintColor: Colors.transparent,
      elevation: 0,
      foregroundColor: Palette.fg,
      iconTheme: IconThemeData(color: Palette.dim),
      titleTextStyle: TextStyle(
        fontFamily: 'Ubuntu',
        fontSize: 20,
        fontWeight: FontWeight.w500,
        color: Palette.fg,
      ),
    ),
    inputDecorationTheme: InputDecorationTheme(
      filled: true,
      fillColor: Palette.bg2,
      labelStyle: const TextStyle(color: Palette.dim),
      helperStyle: const TextStyle(color: Palette.dim),
      border: OutlineInputBorder(
        borderRadius: BorderRadius.circular(16),
        borderSide: const BorderSide(color: Palette.line),
      ),
      enabledBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(16),
        borderSide: const BorderSide(color: Palette.line),
      ),
      focusedBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(16),
        borderSide: const BorderSide(color: Palette.cyan),
      ),
    ),
    snackBarTheme: const SnackBarThemeData(
      backgroundColor: Palette.bg2,
      contentTextStyle: TextStyle(fontFamily: 'Ubuntu', color: Palette.fg),
      behavior: SnackBarBehavior.floating,
    ),
    popupMenuTheme: PopupMenuThemeData(
      color: Palette.bg2,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(14),
        side: const BorderSide(color: Palette.line),
      ),
    ),
    progressIndicatorTheme: const ProgressIndicatorThemeData(
      color: Palette.cyan,
    ),
  );
}

class Backdrop extends StatelessWidget {
  const Backdrop({super.key, required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) {
    return DecoratedBox(
      decoration: const BoxDecoration(
        color: Palette.bg,
        gradient: RadialGradient(
          center: Alignment(-0.4, -1.05),
          radius: 1.5,
          colors: [Palette.glow, Color(0xFF101C47), Palette.bg],
          stops: [0, 0.32, 0.7],
        ),
      ),
      child: child,
    );
  }
}

class Screen extends StatelessWidget {
  const Screen({
    super.key,
    this.title,
    required this.body,
    this.foot = const [],
    this.actions = const [],
    this.closing = false,
  });

  final String? title;
  final Widget body;
  final List<Widget> foot;
  final List<Widget> actions;
  final bool closing;

  @override
  Widget build(BuildContext context) {
    return Backdrop(
      child: Scaffold(
        appBar: title == null
            ? null
            : AppBar(
                title: Text(title!),
                actions: actions,
                leading: Navigator.of(context).canPop()
                    ? IconButton(
                        tooltip: closing ? 'Close' : 'Back',
                        icon: Icon(closing ? Icons.close : Icons.arrow_back),
                        onPressed: () => Navigator.of(context).maybePop(),
                      )
                    : null,
              ),
        body: SafeArea(
          top: title == null,
          child: Column(
            children: [
              Expanded(child: body),
              if (foot.isNotEmpty)
                Padding(
                  padding: const EdgeInsets.fromLTRB(24, 12, 24, 20),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    spacing: 12,
                    children: foot,
                  ),
                ),
            ],
          ),
        ),
      ),
    );
  }
}

class Primary extends StatelessWidget {
  const Primary(this.label, {super.key, this.icon, this.onPressed});

  final String label;
  final IconData? icon;
  final VoidCallback? onPressed;

  @override
  Widget build(BuildContext context) {
    final enabled = onPressed != null;
    return Opacity(
      opacity: enabled ? 1 : 0.4,
      child: DecoratedBox(
        decoration: BoxDecoration(
          borderRadius: BorderRadius.circular(20),
          gradient: const LinearGradient(colors: [Palette.blue, Palette.sky]),
          boxShadow: enabled
              ? [
                  BoxShadow(
                    color: Palette.blue.withValues(alpha: 0.35),
                    blurRadius: 18,
                    offset: const Offset(0, 6),
                  ),
                ]
              : null,
        ),
        child: _Press(
          onPressed: onPressed,
          label: label,
          icon: icon,
          color: Colors.white,
        ),
      ),
    );
  }
}

class Secondary extends StatelessWidget {
  const Secondary(
    this.label, {
    super.key,
    this.icon,
    this.onPressed,
    this.fill,
  });

  final String label;
  final IconData? icon;
  final VoidCallback? onPressed;
  final Color? fill;

  @override
  Widget build(BuildContext context) {
    return Opacity(
      opacity: onPressed != null ? 1 : 0.4,
      child: DecoratedBox(
        decoration: BoxDecoration(
          color: fill,
          borderRadius: BorderRadius.circular(20),
          border: Border.all(color: Palette.line, width: 1.5),
        ),
        child: _Press(
          onPressed: onPressed,
          label: label,
          icon: icon,
          color: Palette.fg,
        ),
      ),
    );
  }
}

class _Press extends StatelessWidget {
  const _Press({
    required this.onPressed,
    required this.label,
    required this.icon,
    required this.color,
  });

  final VoidCallback? onPressed;
  final String label;
  final IconData? icon;
  final Color color;

  @override
  Widget build(BuildContext context) {
    return Material(
      type: MaterialType.transparency,
      child: InkWell(
        borderRadius: BorderRadius.circular(20),
        onTap: onPressed,
        child: Padding(
          padding: const EdgeInsets.symmetric(vertical: 17, horizontal: 16),
          child: Row(
            mainAxisAlignment: MainAxisAlignment.center,
            mainAxisSize: MainAxisSize.min,
            spacing: 10,
            children: [
              if (icon != null) Icon(icon, size: 20, color: color),
              Flexible(
                child: Text(
                  label,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    color: color,
                    fontWeight: FontWeight.w500,
                    fontSize: 17,
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class Ghost extends StatelessWidget {
  const Ghost(this.label, {super.key, this.icon, this.onPressed});

  final String label;
  final IconData? icon;
  final VoidCallback? onPressed;

  @override
  Widget build(BuildContext context) {
    return TextButton.icon(
      onPressed: onPressed,
      style: TextButton.styleFrom(
        foregroundColor: Palette.cyan,
        textStyle: const TextStyle(
          fontFamily: 'Ubuntu',
          fontSize: 15,
          fontWeight: FontWeight.w500,
        ),
      ),
      icon: icon == null ? null : Icon(icon, size: 18),
      label: Text(label),
    );
  }
}

class Panel extends StatelessWidget {
  const Panel({
    super.key,
    required this.child,
    this.padding = const EdgeInsets.symmetric(horizontal: 16, vertical: 14),
    this.onTap,
  });

  final Widget child;
  final EdgeInsets padding;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    return Material(
      color: Palette.bg2,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(20),
        side: const BorderSide(color: Palette.line),
      ),
      clipBehavior: Clip.antiAlias,
      child: InkWell(
        onTap: onTap,
        child: Padding(padding: padding, child: child),
      ),
    );
  }
}

class Tile extends StatelessWidget {
  const Tile(
    this.icon, {
    super.key,
    this.size = 44,
    this.color = Palette.tile,
    this.iconColor = Palette.cyan,
  });

  final IconData icon;
  final double size;
  final Color color;
  final Color iconColor;

  @override
  Widget build(BuildContext context) {
    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(
        color: color,
        borderRadius: BorderRadius.circular(size * 0.28),
      ),
      child: Icon(icon, size: size * 0.5, color: iconColor),
    );
  }
}

IconData iconFor(String name) {
  final ext = name.contains('.') ? name.split('.').last.toLowerCase() : '';
  return switch (ext) {
    'jpg' ||
    'jpeg' ||
    'png' ||
    'gif' ||
    'webp' ||
    'heic' ||
    'bmp' => Icons.image_outlined,
    'mp4' || 'mov' || 'mkv' || 'webm' || 'avi' => Icons.movie_outlined,
    'mp3' ||
    'flac' ||
    'ogg' ||
    'wav' ||
    'm4a' ||
    'opus' => Icons.audiotrack_outlined,
    'zip' ||
    'tar' ||
    'gz' ||
    'xz' ||
    '7z' ||
    'rar' => Icons.inventory_2_outlined,
    'pdf' => Icons.picture_as_pdf_outlined,
    'apk' => Icons.android,
    _ => Icons.description_outlined,
  };
}

class Item extends StatelessWidget {
  const Item({
    super.key,
    required this.icon,
    required this.name,
    required this.sub,
    this.trailing,
    this.onTap,
    this.boxed = true,
  });

  final IconData icon;
  final String name;
  final String sub;
  final Widget? trailing;
  final VoidCallback? onTap;
  final bool boxed;

  @override
  Widget build(BuildContext context) {
    final row = Row(
      spacing: 14,
      children: [
        Tile(icon),
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            spacing: 2,
            children: [
              Text(
                name,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: const TextStyle(
                  fontSize: 16,
                  fontWeight: FontWeight.w500,
                ),
              ),
              Text(sub, style: subStyle),
            ],
          ),
        ),
        ?trailing,
      ],
    );
    if (boxed) {
      return Panel(
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
        onTap: onTap,
        child: row,
      );
    }
    return InkWell(
      borderRadius: BorderRadius.circular(16),
      onTap: onTap,
      child: Padding(
        padding: const EdgeInsets.symmetric(vertical: 8, horizontal: 4),
        child: row,
      ),
    );
  }
}

const subStyle = TextStyle(color: Palette.dim, fontSize: 13.5);

class Label extends StatelessWidget {
  const Label(this.text, {super.key});

  final String text;

  @override
  Widget build(BuildContext context) {
    return Text(
      text.toUpperCase(),
      style: const TextStyle(
        color: Palette.dim,
        fontSize: 13,
        letterSpacing: 1,
        fontWeight: FontWeight.w500,
      ),
    );
  }
}

class Pill extends StatelessWidget {
  const Pill(this.text, {super.key, this.icon});

  final String text;
  final IconData? icon;

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 5),
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(99),
        border: Border.all(color: Palette.pillLine),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        spacing: 6,
        children: [
          if (icon != null) Icon(icon, size: 15, color: Palette.cyan),
          Flexible(
            child: Text(
              text,
              overflow: TextOverflow.ellipsis,
              style: const TextStyle(color: Palette.cyan, fontSize: 13),
            ),
          ),
        ],
      ),
    );
  }
}

class Code extends StatelessWidget {
  const Code(this.text, {super.key, this.size = 15});

  final String text;
  final double size;

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 5),
      decoration: BoxDecoration(
        color: Palette.code,
        borderRadius: BorderRadius.circular(8),
      ),
      child: SelectableText(
        text,
        style: TextStyle(fontFamily: mono, fontSize: size, color: Palette.cyan),
      ),
    );
  }
}

class Bar extends StatelessWidget {
  const Bar(this.value, {super.key});

  final double? value;

  @override
  Widget build(BuildContext context) {
    return ClipRRect(
      borderRadius: BorderRadius.circular(4),
      child: SizedBox(
        height: 8,
        child: value == null
            ? const LinearProgressIndicator(
                backgroundColor: Palette.track,
                color: Palette.cyan,
              )
            : Stack(
                fit: StackFit.expand,
                children: [
                  const Positioned.fill(
                    child: ColoredBox(color: Palette.track),
                  ),
                  FractionallySizedBox(
                    alignment: Alignment.centerLeft,
                    heightFactor: 1,
                    widthFactor: value!.clamp(0, 1),
                    child: const DecoratedBox(
                      decoration: BoxDecoration(
                        gradient: LinearGradient(
                          colors: [Palette.blue, Palette.cyan],
                        ),
                      ),
                    ),
                  ),
                ],
              ),
      ),
    );
  }
}

class Ring extends StatelessWidget {
  const Ring({super.key, required this.value, required this.child});

  final double? value;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    return SizedBox.square(
      dimension: 210,
      child: Stack(
        alignment: Alignment.center,
        children: [
          Positioned.fill(
            child: value == null
                ? const Padding(
                    padding: EdgeInsets.all(6),
                    child: CircularProgressIndicator(
                      strokeWidth: 12,
                      color: Palette.cyan,
                      backgroundColor: Palette.track,
                      strokeCap: StrokeCap.round,
                    ),
                  )
                : CustomPaint(painter: _RingPainter(value!.clamp(0, 1))),
          ),
          child,
        ],
      ),
    );
  }
}

class _RingPainter extends CustomPainter {
  _RingPainter(this.value);

  final double value;

  @override
  void paint(Canvas canvas, Size size) {
    const stroke = 12.0;
    final rect = Offset.zero & size;
    final arc = rect.deflate(stroke / 2 + 6);
    canvas.drawArc(
      arc,
      0,
      2 * math.pi,
      false,
      Paint()
        ..color = Palette.track
        ..style = PaintingStyle.stroke
        ..strokeWidth = stroke,
    );
    if (value <= 0) return;
    final sweep = 2 * math.pi * value;
    canvas.drawArc(
      arc,
      -math.pi / 2,
      sweep,
      false,
      Paint()
        ..shader = SweepGradient(
          startAngle: 0,
          endAngle: 2 * math.pi,
          transform: const GradientRotation(-math.pi / 2),
          colors: const [Palette.blue, Palette.cyan, Palette.blue],
        ).createShader(arc)
        ..style = PaintingStyle.stroke
        ..strokeCap = StrokeCap.round
        ..strokeWidth = stroke,
    );
  }

  @override
  bool shouldRepaint(_RingPainter old) => old.value != value;
}

class Check extends StatelessWidget {
  const Check({super.key, this.failed = false});

  final bool failed;

  @override
  Widget build(BuildContext context) {
    final color = failed ? Palette.error : Palette.ok;
    return Container(
      width: 124,
      height: 124,
      decoration: BoxDecoration(
        shape: BoxShape.circle,
        border: Border.all(color: color, width: 3),
        gradient: RadialGradient(
          colors: [
            color.withValues(alpha: 0.25),
            color.withValues(alpha: 0.05),
          ],
        ),
        boxShadow: [
          BoxShadow(color: color.withValues(alpha: 0.25), blurRadius: 30),
        ],
      ),
      child: Icon(failed ? Icons.close : Icons.check, size: 60, color: color),
    );
  }
}

class Dot extends StatelessWidget {
  const Dot({super.key, this.color = Palette.ok});

  final Color color;

  @override
  Widget build(BuildContext context) {
    return Container(
      width: 9,
      height: 9,
      decoration: BoxDecoration(
        shape: BoxShape.circle,
        color: color,
        boxShadow: [BoxShadow(color: color, blurRadius: 8)],
      ),
    );
  }
}

class Centered extends StatelessWidget {
  const Centered({super.key, required this.children});

  final List<Widget> children;

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, box) => SingleChildScrollView(
        padding: const EdgeInsets.symmetric(horizontal: 24),
        child: ConstrainedBox(
          constraints: BoxConstraints(minHeight: box.maxHeight),
          child: Column(
            mainAxisAlignment: MainAxisAlignment.center,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: children,
          ),
        ),
      ),
    );
  }
}
