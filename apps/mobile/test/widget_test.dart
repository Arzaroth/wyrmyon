import 'package:flutter_test/flutter_test.dart';
import 'package:wyrmyon/main.dart';

void main() {
  testWidgets('Home offers sending and receiving', (tester) async {
    await tester.pumpWidget(const WyrmyonApp());
    expect(find.text('Send'), findsOneWidget);
    expect(find.text('Receive'), findsOneWidget);
  });
}
