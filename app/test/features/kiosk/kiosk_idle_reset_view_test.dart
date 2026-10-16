import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/features/kiosk/app/kiosk_providers.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/controllers/kiosk_controller.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/pages/kiosk_page.dart';
import 'package:xchange_kiosk/features/setup/app/setup_providers.dart';
import 'package:xchange_kiosk/l10n/kiosk_l10n.dart';

import 'kiosk_controller_test.dart' show FakeRepository;

KioskL10n _l10nWithJa() => KioskL10n.fromJson({
  'langs': [
    {'code': 'en', 'lang': 'en', 'name': 'English', 'search': 'english'},
    {'code': 'th', 'lang': 'th', 'name': 'ไทย', 'search': 'thai ไทย'},
    {'code': 'ja', 'lang': 'ja', 'name': '日本語', 'search': 'japanese 日本'},
  ],
  'text': {
    'en': {
      'welcome': 'Welcome',
      'closed': 'closed-en',
      'errGeneric': 'generic-en',
      'quoteExpired': 'expired-en',
      'statusError': 'status-error-en',
      'title': 'Exchange',
      'subtitle': 'Choose currency',
    },
    'th': {'welcome': 'ยินดีต้อนรับ', 'closed': 'closed-th'},
    'ja': {'welcome': 'ようこそ', 'closed': 'closed-ja'},
  },
  'errors': <String, String>{},
});

void main() {
  late FakeRepository server;
  late ProviderContainer container;

  Finder searchFieldFinder() =>
      find.widgetWithText(TextField, 'Search language / ภาษา / 国家');

  Finder langDropdownFinder() => find.byWidgetPredicate(
    (w) =>
        w is DropdownButtonFormField<String> &&
        w.key is ValueKey<String> &&
        (w.key! as ValueKey<String>).value.startsWith('lang-'),
  );

  String searchText(WidgetTester tester) =>
      tester.widget<TextField>(searchFieldFinder()).controller!.text;

  List<String> dropdownItems(WidgetTester tester) => tester
      .widget<DropdownButton<String>>(
        find.descendant(
          of: langDropdownFinder(),
          matching: find.byType(DropdownButton<String>),
        ),
      )
      .items!
      .map((item) => item.value!)
      .toList();

  Future<KioskController> pumpKiosk(
    WidgetTester tester, {
    Size size = const Size(1280, 900),
  }) async {
    tester.view.physicalSize = size;
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);
    container = ProviderContainer(
      overrides: [
        kioskRepositoryProvider.overrideWithValue(server),
        kioskL10nProvider.overrideWithValue(_l10nWithJa()),
        deviceLanguagesProvider.overrideWithValue(const ['en']),
        setupAvailableProvider.overrideWithValue(true),
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

  setUp(() => server = FakeRepository());

  testWidgets(
    'row 1: Type "日本" in the language search, no pick, idle reset (2 min) -> Search box empty; full language list',
    (tester) async {
      await pumpKiosk(tester);

      expect(container.read(kioskControllerProvider).languageCode, 'en');
      expect(
        container.read(kioskControllerProvider).screen,
        KioskScreen.select,
      );

      // Customer types in search box without picking a language
      await tester.enterText(searchFieldFinder(), '日本');
      await tester.pump();
      expect(searchText(tester), '日本');
      expect(dropdownItems(tester), ['ja']);

      // 2 minutes idle timeout triggers reset
      await tester.pump(const Duration(minutes: 2));
      await tester.pump();

      // Screen is still select and language is still en, but resetGeneration changed.
      // Search box must be cleared and dropdown list restored.
      expect(searchText(tester), '');
      expect(dropdownItems(tester), ['en', 'th', 'ja']);

      await finish(tester);
    },
  );

  testWidgets(
    'row 2: Scroll the select page down, idle reset -> Scroll offset back to the top',
    (tester) async {
      // Use a smaller portrait viewport so the select page content overflows and scrolls.
      await pumpKiosk(tester, size: const Size(600, 500));

      final scrollableFinder = find.byType(SingleChildScrollView);
      expect(scrollableFinder, findsOneWidget);

      final scrollable = find
          .descendant(of: scrollableFinder, matching: find.byType(Scrollable))
          .first;
      final initialScrollState = tester.state<ScrollableState>(scrollable);
      expect(initialScrollState.position.pixels, 0.0);

      // Customer scrolls down
      await tester.drag(scrollableFinder, const Offset(0, -250));
      await tester.pump();

      final scrolledState = tester.state<ScrollableState>(scrollable);
      expect(scrolledState.position.pixels, greaterThan(100.0));

      // 2 minutes idle timeout triggers reset
      await tester.pump(const Duration(minutes: 2));
      await tester.pump();

      // Scroll position must be reset back to the top (0.0)
      final resetScrollState = tester.state<ScrollableState>(scrollable);
      expect(resetScrollState.position.pixels, 0.0);

      await finish(tester);
    },
  );
}
