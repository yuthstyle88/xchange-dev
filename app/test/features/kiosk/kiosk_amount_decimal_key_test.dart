import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/features/kiosk/app/kiosk_providers.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/controllers/kiosk_controller.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/pages/kiosk_page.dart';

import 'kiosk_controller_test.dart' show FakeRepository, l10n;

void main() {
  late FakeRepository server;
  late ProviderContainer container;

  Future<KioskController> pumpKiosk(WidgetTester tester) async {
    tester.view.physicalSize = const Size(1280, 1400);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);
    container = ProviderContainer(
      overrides: [
        kioskRepositoryProvider.overrideWithValue(server),
        kioskL10nProvider.overrideWithValue(l10n()),
        deviceLanguagesProvider.overrideWithValue(const ['en']),
      ],
    );
    await tester.pumpWidget(
      UncontrolledProviderScope(
        container: container,
        child: const MaterialApp(home: KioskPage()),
      ),
    );
    final c = container.read(kioskControllerProvider.notifier);
    await c.start();
    await tester.pump();
    return c;
  }

  Future<void> finish(WidgetTester tester) async {
    await tester.pumpWidget(const SizedBox());
    container.dispose();
  }

  TextField amountField(WidgetTester tester) =>
      tester.widget<TextField>(find.byKey(const Key('amount')));

  Future<void> typeKeys(WidgetTester tester, String keys) async {
    final finder = find.byKey(const Key('amount'));
    for (final rune in keys.runes) {
      final ch = String.fromCharCode(rune);
      final current = amountField(tester).controller?.text ?? '';
      await tester.enterText(finder, '$current$ch');
      await tester.pump();
    }
  }

  void expectAmount(WidgetTester tester, String expected) {
    expect(amountField(tester).controller?.text, expected);
    expect(container.read(kioskControllerProvider).amountText, expected);
  }

  setUp(() => server = FakeRepository());

  testWidgets('row 1: typing 1 0 0 , 5 0 produces 100.50', (tester) async {
    await pumpKiosk(tester);
    await typeKeys(tester, '100,50');
    expectAmount(tester, '100.50');
    await finish(tester);
  });

  testWidgets('row 2: typing Eastern Arabic digits ١ ٠ ٠ produces 100', (
    tester,
  ) async {
    await pumpKiosk(tester);
    await typeKeys(tester, '١٠٠');
    expectAmount(tester, '100');
    await finish(tester);
  });

  testWidgets(
    'row 3: typing Arabic decimal separator 1 0 0 ٫ 5 produces 100.5',
    (tester) async {
      await pumpKiosk(tester);
      await typeKeys(tester, '100٫5');
      expectAmount(tester, '100.5');
      await finish(tester);
    },
  );

  testWidgets('row 4: typing Persian digits ۱ ۲ , ۵ produces 12.5', (
    tester,
  ) async {
    await pumpKiosk(tester);
    await typeKeys(tester, '۱۲,۵');
    expectAmount(tester, '12.5');
    await finish(tester);
  });

  testWidgets('row 5: typing ASCII 1 0 0 . 5 0 keeps 100.50', (tester) async {
    await pumpKiosk(tester);
    await typeKeys(tester, '100.50');
    expectAmount(tester, '100.50');
    await finish(tester);
  });

  testWidgets('row 6: typing 123456789 keeps 8-digit limit 12345678', (
    tester,
  ) async {
    await pumpKiosk(tester);
    await typeKeys(tester, '123456789');
    expectAmount(tester, '12345678');
    await finish(tester);
  });

  testWidgets(
    'row 7: typing 1 , 2 , 3 drops second separator and produces 1.23',
    (tester) async {
      await pumpKiosk(tester);
      await typeKeys(tester, '1,2,3');
      expectAmount(tester, '1.23');
      await finish(tester);
    },
  );

  testWidgets('row 8: typing 1 2 , 3 4 5 keeps 2-decimal limit 12.34', (
    tester,
  ) async {
    await pumpKiosk(tester);
    await typeKeys(tester, '12,345');
    expectAmount(tester, '12.34');
    await finish(tester);
  });

  testWidgets('row 9: typing , 5 keeps leading decimal .5 (#171)', (
    tester,
  ) async {
    await pumpKiosk(tester);
    await typeKeys(tester, ',5');
    expectAmount(tester, '.5');
    await finish(tester);
  });
}
