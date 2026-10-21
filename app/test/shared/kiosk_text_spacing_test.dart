import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/app/app.dart';
import 'package:xchange_kiosk/features/queue_machine/presentation/pages/queue_machine_home_page.dart';

void main() {
  testWidgets('queue machine shell title: Thai font fallback and spacing', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(1280, 1600);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);

    final container = ProviderContainer();
    container.read(appLanguageProvider.notifier).state = 'th';

    await tester.pumpWidget(
      UncontrolledProviderScope(container: container, child: const KioskApp()),
    );
    await tester.pumpAndSettle();

    expect(find.byType(QueueMachineHomePage), findsOneWidget);

    final titleFinder = find.byKey(const Key('queue-machine-title'));
    expect(titleFinder, findsOneWidget);

    final richText = tester.widget<RichText>(
      find.descendant(of: titleFinder, matching: find.byType(RichText)).first,
    );
    final style = richText.text.style;

    expect(style?.fontFamilyFallback, contains('NotoSansThai'));
    expect(style?.letterSpacing, anyOf(isNull, 0.0));

    container.dispose();
  });

  testWidgets(
    'queue machine shell title follows app language (ja starts with NotoSansJP)',
    (tester) async {
      tester.view.physicalSize = const Size(1280, 1600);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);

      final container = ProviderContainer();
      container.read(appLanguageProvider.notifier).state = 'ja';

      await tester.pumpWidget(
        UncontrolledProviderScope(
          container: container,
          child: const KioskApp(),
        ),
      );
      await tester.pumpAndSettle();

      expect(find.byType(QueueMachineHomePage), findsOneWidget);

      final titleFinder = find.byKey(const Key('queue-machine-title'));
      expect(titleFinder, findsOneWidget);

      final richText = tester.widget<RichText>(
        find.descendant(of: titleFinder, matching: find.byType(RichText)).first,
      );
      final style = richText.text.style;

      expect(style?.fontFamilyFallback, isNotNull);
      expect(style?.fontFamilyFallback?.first, 'NotoSansJP');

      container.dispose();
    },
  );
}
