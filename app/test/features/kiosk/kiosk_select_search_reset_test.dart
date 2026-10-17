import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/features/kiosk/app/kiosk_providers.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/controllers/kiosk_controller.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/pages/kiosk_page.dart';

import 'kiosk_controller_test.dart' show FakeRepository, l10n;

/// #2479: the currency search on screen 1 must not survive a reset.
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

  // The test texts have no 'searchPlaceholder', so the hint is the key.
  final searchBox = find.widgetWithText(TextField, 'searchPlaceholder');
  final amountBox = find.byKey(const Key('amount'));

  String searchText(WidgetTester tester) =>
      tester.widget<TextField>(searchBox).controller!.text;
  String amountText(WidgetTester tester) =>
      tester.widget<TextField>(amountBox).controller!.text;

  /// Lets the 2-minute idle timer fire and the reset's rates answer land.
  Future<void> idleReset(WidgetTester tester) async {
    final before = container.read(kioskControllerProvider).resetGeneration;
    await tester.pump(const Duration(minutes: 2, seconds: 1));
    await tester.pump();
    expect(
      container.read(kioskControllerProvider).resetGeneration,
      greaterThan(before),
      reason: 'the idle reset must have fired',
    );
  }

  void expectAllChips() {
    expect(find.byKey(const Key('currency-USD')), findsOneWidget);
    expect(find.byKey(const Key('currency-JPY')), findsOneWidget);
    expect(find.text('noCurrencies'), findsNothing);
  }

  setUp(() => server = FakeRepository());

  testWidgets(
    'row 1: type a search, no amount, idle reset fires -> search empty, all chips',
    (tester) async {
      await pumpKiosk(tester);

      await tester.enterText(searchBox, 'xyz');
      await tester.pump();
      expect(find.text('noCurrencies'), findsOneWidget);

      await idleReset(tester);

      expect(searchText(tester), '');
      expectAllChips();
      await finish(tester);
    },
  );

  testWidgets(
    'row 2: type a search and an amount, idle reset fires -> both boxes empty',
    (tester) async {
      await pumpKiosk(tester);

      await tester.enterText(searchBox, 'yen');
      await tester.enterText(amountBox, '100');
      await tester.pump();
      expect(find.byKey(const Key('currency-USD')), findsNothing);

      await idleReset(tester);

      expect(searchText(tester), '');
      expect(amountText(tester), '');
      expectAllChips();
      await finish(tester);
    },
  );

  testWidgets(
    'row 3: type a search, type then delete an amount (no reset) -> search kept',
    (tester) async {
      await pumpKiosk(tester);

      await tester.enterText(searchBox, 'yen');
      await tester.enterText(amountBox, '100');
      await tester.pump();
      await tester.enterText(amountBox, '');
      await tester.pump();

      expect(container.read(kioskControllerProvider).amountText, '');
      expect(searchText(tester), 'yen');
      expect(find.byKey(const Key('currency-JPY')), findsOneWidget);
      expect(find.byKey(const Key('currency-USD')), findsNothing);
      await finish(tester);
    },
  );

  testWidgets(
    'edge: a setup Save (kiosk start() -> reset()) with a search and no amount '
    'clears the search',
    (tester) async {
      final c = await pumpKiosk(tester);

      await tester.enterText(searchBox, 'xyz');
      await tester.pump();
      expect(find.text('noCurrencies'), findsOneWidget);

      // SetupController's Save calls exactly this (setup_controller.dart).
      await c.start();
      await tester.pump();

      expect(searchText(tester), '');
      expectAllChips();
      await finish(tester);
    },
  );
}
