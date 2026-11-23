import 'package:flutter/material.dart';
import 'package:mobile_scanner/mobile_scanner.dart';

import '../app.dart';
import '../engine.dart';
import '../ui.dart';
import 'type_code.dart';

enum Purpose { send, receive }

class ScanPage extends StatefulWidget {
  const ScanPage({super.key, required this.purpose});

  final Purpose purpose;

  static Route<String> route(Purpose purpose) =>
      MaterialPageRoute(builder: (_) => ScanPage(purpose: purpose));

  @override
  State<ScanPage> createState() => _ScanPageState();
}

class _ScanPageState extends State<ScanPage> {
  String? _problem;
  bool _done = false;

  void _detect(BarcodeCapture capture) {
    if (_done) return;
    final engine = Services.of(context).engine;
    for (final barcode in capture.barcodes) {
      final text = barcode.rawValue;
      if (text == null) continue;
      final scanned = engine.scan(text);
      final wanted = widget.purpose == Purpose.send
          ? Direction.send
          : Direction.receive;
      if (scanned == null) {
        setState(() => _problem = 'This is not a wormhole code');
      } else if (scanned.direction != wanted) {
        setState(
          () => _problem = wanted == Direction.send
              ? 'This code sends to the phone: use Receive'
              : 'This code waits for files: use Send',
        );
      } else {
        _done = true;
        Navigator.of(context).pop(scanned.code);
        return;
      }
    }
  }

  Future<void> _type() async {
    final navigator = Navigator.of(context);
    final code = await navigator.push(
      TypeCodePage.route(purpose: widget.purpose, fromScanner: true),
    );
    if (code != null) navigator.pop(code);
  }

  @override
  Widget build(BuildContext context) {
    final command = widget.purpose == Purpose.send
        ? 'wyrm receive --new --qr'
        : 'wyrm send --qr FILE';
    const veil = Color(0xD9080C1E);
    return Backdrop(
      child: Scaffold(
        extendBodyBehindAppBar: true,
        appBar: AppBar(
          leading: IconButton(
            tooltip: 'Close',
            icon: const Icon(Icons.close, color: Palette.fg),
            onPressed: () => Navigator.of(context).maybePop(),
          ),
          title: const Text("Scan the computer's code"),
        ),
        body: Stack(
          fit: StackFit.expand,
          children: [
            MobileScanner(
              onDetect: _detect,
              errorBuilder: (context, error) => Center(
                child: Padding(
                  padding: const EdgeInsets.all(32),
                  child: Text(
                    'The camera is not available: '
                    '${error.errorDetails?.message ?? error.errorCode.name}',
                    textAlign: TextAlign.center,
                    style: subStyle,
                  ),
                ),
              ),
            ),
            const IgnorePointer(child: _Reticle()),
            SafeArea(
              child: Align(
                alignment: Alignment.bottomCenter,
                child: Padding(
                  padding: const EdgeInsets.fromLTRB(24, 0, 24, 20),
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    spacing: 12,
                    children: [
                      if (_problem case final problem?)
                        Center(
                          child: Container(
                            padding: const EdgeInsets.symmetric(
                              horizontal: 14,
                              vertical: 8,
                            ),
                            decoration: BoxDecoration(
                              color: veil,
                              borderRadius: BorderRadius.circular(99),
                              border: Border.all(color: Palette.error),
                            ),
                            child: Text(
                              problem,
                              style: const TextStyle(color: Palette.error),
                            ),
                          ),
                        ),
                      Container(
                        padding: const EdgeInsets.all(16),
                        decoration: BoxDecoration(
                          color: veil,
                          borderRadius: BorderRadius.circular(20),
                          border: Border.all(color: Palette.line),
                        ),
                        child: Column(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          spacing: 8,
                          children: [
                            const Text(
                              'On your computer, run',
                              style: subStyle,
                            ),
                            Code(command),
                          ],
                        ),
                      ),
                      Secondary(
                        'Type the code',
                        icon: Icons.keyboard_outlined,
                        fill: veil,
                        onPressed: _type,
                      ),
                    ],
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

class _Reticle extends StatefulWidget {
  const _Reticle();

  @override
  State<_Reticle> createState() => _ReticleState();
}

class _ReticleState extends State<_Reticle>
    with SingleTickerProviderStateMixin {
  late final _sweep = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 1800),
  )..repeat(reverse: true);

  @override
  void dispose() {
    _sweep.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Stack(
      fit: StackFit.expand,
      children: [
        const CustomPaint(painter: _Veil()),
        _frame(),
      ],
    );
  }

  Widget _frame() {
    return Align(
      alignment: const Alignment(0, -0.2),
      child: SizedBox.square(
        dimension: 240,
        child: AnimatedBuilder(
          animation: _sweep,
          builder: (context, _) => CustomPaint(
            painter: _ReticlePainter(Curves.easeInOut.transform(_sweep.value)),
          ),
        ),
      ),
    );
  }
}

class _Veil extends CustomPainter {
  const _Veil();

  @override
  void paint(Canvas canvas, Size size) {
    const side = 240.0;
    final top = (size.height - side) / 2 * (1 - 0.2);
    final hole = RRect.fromRectAndRadius(
      Rect.fromLTWH((size.width - side) / 2, top, side, side),
      const Radius.circular(26),
    );
    final veil = Path()
      ..fillType = PathFillType.evenOdd
      ..addRect(Offset.zero & size)
      ..addRRect(hole);
    canvas.drawPath(veil, Paint()..color = const Color(0x8C050816));
  }

  @override
  bool shouldRepaint(_Veil old) => false;
}

class _ReticlePainter extends CustomPainter {
  _ReticlePainter(this.at);

  final double at;

  @override
  void paint(Canvas canvas, Size size) {
    const arm = 44.0;
    const radius = 26.0;
    final corners = Paint()
      ..color = Palette.cyan
      ..style = PaintingStyle.stroke
      ..strokeWidth = 5
      ..strokeCap = StrokeCap.round;
    final w = size.width;
    final h = size.height;
    final path = Path()
      ..moveTo(0, arm)
      ..lineTo(0, radius)
      ..arcToPoint(
        const Offset(radius, 0),
        radius: const Radius.circular(radius),
      )
      ..lineTo(arm, 0)
      ..moveTo(w - arm, 0)
      ..lineTo(w - radius, 0)
      ..arcToPoint(Offset(w, radius), radius: const Radius.circular(radius))
      ..lineTo(w, arm)
      ..moveTo(w, h - arm)
      ..lineTo(w, h - radius)
      ..arcToPoint(Offset(w - radius, h), radius: const Radius.circular(radius))
      ..lineTo(w - arm, h)
      ..moveTo(arm, h)
      ..lineTo(radius, h)
      ..arcToPoint(Offset(0, h - radius), radius: const Radius.circular(radius))
      ..lineTo(0, h - arm);
    canvas.drawPath(path, corners);
    final y = 24 + (h - 48) * at;
    final line = Rect.fromLTWH(24, y - 1.5, w - 48, 3);
    canvas
      ..drawRect(
        line.inflate(6),
        Paint()
          ..color = Palette.cyan.withValues(alpha: 0.35)
          ..maskFilter = const MaskFilter.blur(BlurStyle.normal, 8),
      )
      ..drawRect(line, Paint()..color = Palette.cyan);
  }

  @override
  bool shouldRepaint(_ReticlePainter old) => old.at != at;
}
