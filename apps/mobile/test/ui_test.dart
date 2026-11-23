import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wyrmyon/ui.dart';

void main() {
  testWidgets('a bar fills its share of the width, at full height', (
    tester,
  ) async {
    await tester.pumpWidget(
      const Directionality(
        textDirection: TextDirection.ltr,
        child: Center(child: SizedBox(width: 200, child: Bar(0.25))),
      ),
    );
    final fill = tester.getSize(
      find.descendant(
        of: find.byType(FractionallySizedBox),
        matching: find.byType(DecoratedBox),
      ),
    );
    expect(fill, const Size(50, 8));
  });
}
