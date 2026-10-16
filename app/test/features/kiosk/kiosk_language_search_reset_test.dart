import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:go_router/go_router.dart';
import 'package:xchange_kiosk/app/router/app_router.dart';
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

  String? dropdownValue(WidgetTester tester) => tester
      .widget<DropdownButton<String>>(
        find.descendant(
          of: langDropdownFinder(),
          matching: find.byType(DropdownButton<String>),
        ),
      )
      .value;

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

  Future<void> selectLanguageFromDropdown(
    WidgetTester tester,
    String code,
  ) async {
    final dropdown = tester.widget<DropdownButton<String>>(
      find.descendant(
        of: langDropdownFinder(),
        matching: find.byType(DropdownButton<String>),
      ),
    );
    dropdown.onChanged!(code);
    await tester.pump();
  }

  Future<KioskController> pumpKiosk(
    WidgetTester tester, {
    bool withRouter = false,
  }) async {
    tester.view.physicalSize = const Size(1280, 900);
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
    if (withRouter) {
      final router = GoRouter(
        initialLocation: '/',
        routes: [
          GoRoute(path: '/', builder: (context, state) => const KioskPage()),
          GoRoute(
            path: setupPath,
            builder: (context, state) =>
                const Scaffold(body: Text('setup-screen')),
          ),
        ],
      );
      await tester.pumpWidget(
        UncontrolledProviderScope(
          container: container,
          child: MaterialApp.router(routerConfig: router),
        ),
      );
    } else {
      await tester.pumpWidget(
        UncontrolledProviderScope(
          container: container,
          child: const MaterialApp(home: KioskPage()),
        ),
      );
    }
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

  testWidgets('row 1: Type "日本", pick Japanese, idle reset -> search box '
      'empty, dropdown shows device-default language, full list', (
    tester,
  ) async {
    await pumpKiosk(tester);

    await tester.enterText(searchFieldFinder(), '日本');
    await tester.pump();
    expect(searchText(tester), '日本');
    expect(dropdownItems(tester), ['ja']);

    await selectLanguageFromDropdown(tester, 'ja');
    expect(container.read(kioskControllerProvider).languageCode, 'ja');

    // Even if the customer also leaves text in the search box before walking
    // away, the idle reset back to the device language clears it.
    await tester.enterText(searchFieldFinder(), '日本');
    await tester.pump();

    await tester.pump(const Duration(minutes: 2));
    await tester.pump();

    expect(container.read(kioskControllerProvider).languageCode, 'en');
    expect(searchText(tester), '');
    expect(dropdownValue(tester), 'en');
    expect(dropdownItems(tester), ['en', 'th', 'ja']);
    await finish(tester);
  });

  testWidgets('row 2: Pick a language from a filtered list -> query cleared, '
      'full list with the pick selected', (tester) async {
    await pumpKiosk(tester);

    await tester.enterText(searchFieldFinder(), '日本');
    await tester.pump();
    expect(searchText(tester), '日本');
    expect(dropdownItems(tester), ['ja']);
    expect(dropdownValue(tester), isNull);

    await selectLanguageFromDropdown(tester, 'ja');

    expect(searchText(tester), '');
    expect(dropdownValue(tester), 'ja');
    expect(dropdownItems(tester), ['en', 'th', 'ja']);
    await finish(tester);
  });

  testWidgets('row 3: Done screen times out -> reset clears search box for the '
      'next customer', (tester) async {
    final c = await pumpKiosk(tester);

    await tester.enterText(searchFieldFinder(), '日本');
    await tester.pump();
    expect(searchText(tester), '日本');

    c
      ..setCurrency('USD')
      ..setAmount('100')
      ..setPaymentMethod('Cash');
    await c.submitQuote();
    await tester.pump();
    expect(container.read(kioskControllerProvider).screen, KioskScreen.quote);
    expect(searchText(tester), '');

    // Also verify that if a query is typed on the done screen before it times
    // out, the reset back to the select screen clears it for the next customer.
    await tester.pump(const Duration(seconds: KioskController.reviewSeconds));
    await c.confirm();
    await tester.pump();
    expect(container.read(kioskControllerProvider).screen, KioskScreen.waiting);

    server.serverStatus = 'COMPLETED';
    server.receiptImage =
        'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==';
    await tester.pump(const Duration(seconds: 5));
    await tester.pump(const Duration(seconds: 3));
    expect(container.read(kioskControllerProvider).screen, KioskScreen.receipt);

    await tester.enterText(searchFieldFinder(), '日本');
    await tester.pump();
    expect(searchText(tester), '日本');

    await tester.pump(KioskController.restartAfterDone);
    await tester.pump();

    expect(container.read(kioskControllerProvider).screen, KioskScreen.select);
    expect(searchText(tester), '');
    expect(dropdownValue(tester), 'en');
    expect(dropdownItems(tester), ['en', 'th', 'ja']);
    await finish(tester);
  });

  testWidgets('row 4: Idle reset returns the language to the device default '
      'and clears the language search box', (tester) async {
    await pumpKiosk(tester);

    await tester.enterText(searchFieldFinder(), 'ไทย');
    await tester.pump();
    await selectLanguageFromDropdown(tester, 'th');
    expect(container.read(kioskControllerProvider).languageCode, 'th');
    expect(searchText(tester), '');

    await tester.enterText(searchFieldFinder(), 'ไทย');
    await tester.pump();

    await tester.pump(const Duration(minutes: 2));
    await tester.pump();

    expect(container.read(kioskControllerProvider).languageCode, 'en');
    expect(searchText(tester), '');
    expect(dropdownValue(tester), 'en');
    expect(dropdownItems(tester), ['en', 'th', 'ja']);
    await finish(tester);
  });

  testWidgets('row 5: Holding the logo for 10 s opens setup and picking a '
      'language clears the search query', (tester) async {
    await pumpKiosk(tester, withRouter: true);

    await tester.enterText(searchFieldFinder(), '日本');
    await tester.pump();
    await selectLanguageFromDropdown(tester, 'ja');
    expect(searchText(tester), '');
    expect(dropdownItems(tester), ['en', 'th', 'ja']);

    final gesture = await tester.startGesture(
      tester.getCenter(find.byKey(const Key('kiosk-logo'))),
    );
    await tester.pump(const Duration(seconds: 10));
    await gesture.up();
    await tester.pumpAndSettle();

    expect(find.text('setup-screen'), findsOneWidget);
    await finish(tester);
  });

  testWidgets('edge case: Choosing the already-selected language keeps the '
      'query, whereas choosing a different language clears it', (tester) async {
    await pumpKiosk(tester);

    await tester.enterText(searchFieldFinder(), 'en');
    await tester.pump();
    expect(searchText(tester), 'en');

    await selectLanguageFromDropdown(tester, 'en');
    expect(searchText(tester), 'en');

    await tester.enterText(searchFieldFinder(), '日本');
    await tester.pump();
    await selectLanguageFromDropdown(tester, 'ja');
    expect(searchText(tester), '');
    expect(dropdownValue(tester), 'ja');
    expect(dropdownItems(tester), ['en', 'th', 'ja']);
    await finish(tester);
  });
}
