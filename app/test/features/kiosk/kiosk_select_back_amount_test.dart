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

  setUp(() => server = FakeRepository());

  testWidgets(
    'row 1: type 100 -> Review -> Back shows 100, estimate matches it, and Review quotes 100',
    (tester) async {
      final c = await pumpKiosk(tester);
      c
        ..setCurrency('USD')
        ..setPaymentMethod('Cash');
      await tester.pump();

      await tester.enterText(find.byKey(const Key('amount')), '100');
      await tester.pump();

      await tester.tap(find.byKey(const Key('review')));
      await tester.pump();
      expect(container.read(kioskControllerProvider).screen, KioskScreen.quote);

      await tester.tap(find.byKey(const Key('back')));
      await tester.pump();
      expect(
        container.read(kioskControllerProvider).screen,
        KioskScreen.select,
      );

      expect(amountField(tester).controller?.text, '100');
      expect(find.textContaining('3,210.00 THB'), findsWidgets);

      server.lastQuote = null;
      await tester.tap(find.byKey(const Key('review')));
      await tester.pump();
      expect(server.lastQuote?['amount'], '100');

      await finish(tester);
    },
  );

  testWidgets('row 2: Back, then edit to 50 -> Review quotes 50', (
    tester,
  ) async {
    final c = await pumpKiosk(tester);
    c
      ..setCurrency('USD')
      ..setPaymentMethod('Cash');
    await tester.pump();

    await tester.enterText(find.byKey(const Key('amount')), '100');
    await tester.pump();

    await tester.tap(find.byKey(const Key('review')));
    await tester.pump();
    await tester.tap(find.byKey(const Key('back')));
    await tester.pump();

    expect(amountField(tester).controller?.text, '100');

    await tester.enterText(find.byKey(const Key('amount')), '50');
    await tester.pump();
    expect(amountField(tester).controller?.text, '50');

    server.lastQuote = null;
    await tester.tap(find.byKey(const Key('review')));
    await tester.pump();
    expect(server.lastQuote?['amount'], '50');

    await finish(tester);
  });

  testWidgets(
    'row 3: a reset (idle timeout or new customer) leaves the box empty',
    (tester) async {
      final c = await pumpKiosk(tester);
      c
        ..setCurrency('USD')
        ..setPaymentMethod('Cash');
      await tester.pump();

      await tester.enterText(find.byKey(const Key('amount')), '100');
      await tester.pump();
      expect(amountField(tester).controller?.text, '100');

      await c.reset();
      await tester.pump();
      expect(amountField(tester).controller?.text, '');

      await finish(tester);
    },
  );

  testWidgets(
    'row 4: first open, no amount yet -> the box is empty with the hint',
    (tester) async {
      await pumpKiosk(tester);
      final field = amountField(tester);
      expect(field.controller?.text, '');
      expect(field.decoration?.hintText, '0.00');
      await finish(tester);
    },
  );
}
