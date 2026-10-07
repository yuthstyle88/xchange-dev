import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/features/kiosk/app/kiosk_providers.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/controllers/kiosk_controller.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/pages/kiosk_page.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/pages/select_page.dart';

import 'kiosk_controller_test.dart' show FakeRepository, l10n;

// #325: "Let's exchange / Choose the currency…" belongs to step 1 only.
// #326: the error / "counter closed" line sits above the step, not under it.
// Written before the code. The test texts have no 'title' / 'subtitle', so
// KioskL10n.text answers with the key itself and the test finds 'title'.

void main() {
  late FakeRepository server;
  late ProviderContainer container;

  Future<KioskController> pumpKiosk(WidgetTester tester) async {
    tester.view.physicalSize = const Size(1280, 900);
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

  /// The controller keeps a status poller and an idle timer running; take the
  /// page down and dispose the container before the test ends so none is left.
  Future<void> finish(WidgetTester tester) async {
    await tester.pumpWidget(const SizedBox());
    container.dispose();
  }

  setUp(() => server = FakeRepository());

  testWidgets('step 1 shows the title and the subtitle', (tester) async {
    await pumpKiosk(tester);
    expect(container.read(kioskControllerProvider).screen, KioskScreen.select);
    expect(find.text('title'), findsOneWidget);
    expect(find.text('subtitle'), findsOneWidget);
    await finish(tester);
  });

  testWidgets('step 2 does not tell the customer to choose a currency', (
    tester,
  ) async {
    final c = await pumpKiosk(tester);
    c
      ..setCurrency('USD')
      ..setAmount('100')
      ..setPaymentMethod('Cash');
    await c.submitQuote();
    await tester.pump();
    expect(container.read(kioskControllerProvider).screen, KioskScreen.quote);
    expect(find.text('title'), findsNothing);
    expect(find.text('subtitle'), findsNothing);
    // the step label still says where the customer is
    expect(find.text('step2'), findsOneWidget);
    await finish(tester);
  });

  testWidgets('the counter-closed line sits above the step, on screen', (
    tester,
  ) async {
    server.open = false;
    final c = await pumpKiosk(tester);
    c
      ..setCurrency('USD')
      ..setAmount('100')
      ..setPaymentMethod('Cash');
    await c.submitQuote();
    await tester.pump();
    final error = find.byKey(const Key('error'));
    expect(error, findsOneWidget);
    expect(find.text('closed-en'), findsOneWidget);
    final errorTop = tester.getTopLeft(error).dy;
    final stepTop = tester.getTopLeft(find.byType(SelectPage)).dy;
    expect(
      errorTop,
      lessThan(stepTop),
      reason: 'the line comes before the step',
    );
    expect(
      tester.getBottomLeft(error).dy,
      lessThan(900),
      reason: 'visible without scrolling',
    );
    await finish(tester);
  });
}
