// #174 (A3): which screen the app opens on, the ten-second hold on the logo,
// and a whole first-run setup through the screen. Written before the code.
//
// Mutations that must turn this red:
// - no redirect to /setup when nothing is stored   -> Android, nothing stored
// - a /setup route on the web build                -> the web never shows setup
// - a hold shorter than ten seconds                -> 9.9 s is not enough
// - the hold working on the web                    -> the hold does nothing
// - setup not prefilled                            -> … prefilled
// - save enabled before a successful test          -> the save button
import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/app/app.dart';
import 'package:xchange_kiosk/core/endpoint/endpoint_store.dart';
import 'package:xchange_kiosk/core/endpoint/kiosk_endpoint.dart';
import 'package:xchange_kiosk/features/kiosk/app/kiosk_providers.dart';
import 'package:xchange_kiosk/features/kiosk/domain/entities/kiosk_models.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/pages/kiosk_page.dart';
import 'package:xchange_kiosk/features/setup/app/setup_providers.dart';
import 'package:xchange_kiosk/features/setup/l10n/setup_l10n.dart';
import 'package:xchange_kiosk/features/setup/presentation/pages/setup_page.dart';

import '../kiosk/kiosk_controller_test.dart' show FakeRepository, l10n;
import 'setup_controller_test.dart' show MemoryStore;

final _texts = SetupL10n.fromJson(
  jsonDecode(File('assets/l10n/setup.json').readAsStringSync())
      as Map<String, dynamic>,
);

void main() {
  late ProviderContainer container;
  late MemoryStore store;
  late FakeRepository server;
  late List<KioskEndpoint> probed;

  Future<void> pumpApp(
    WidgetTester tester, {
    required bool device,
    KioskEndpoint? stored,
  }) async {
    tester.view.physicalSize = const Size(1280, 900);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);
    store = MemoryStore(stored);
    container = ProviderContainer(
      overrides: [
        endpointStoreProvider.overrideWithValue(store),
        initialKioskEndpointProvider.overrideWithValue(stored),
        setupAvailableProvider.overrideWithValue(device),
        allowInsecureServerProvider.overrideWithValue(false),
        setupL10nProvider.overrideWith((ref) async => _texts),
        setupProbeProvider.overrideWithValue((e) async {
          probed.add(e);
          return RatesModel.fromJson({
            'branchId': e.branchId,
            'branchName': 'Siam Square',
            'open': true,
            'rates': <Object>[],
          });
        }),
        kioskRepositoryProvider.overrideWithValue(server),
        kioskL10nProvider.overrideWithValue(l10n()),
        deviceLanguagesProvider.overrideWithValue(const ['en']),
      ],
    );
    await tester.pumpWidget(
      UncontrolledProviderScope(container: container, child: const KioskApp()),
    );
    await tester.pump();
    await tester.pump();
  }

  /// Take the app down and dispose the container so no timer is left.
  Future<void> finish(WidgetTester tester) async {
    await tester.pumpWidget(const SizedBox());
    container.dispose();
  }

  Future<void> holdLogo(WidgetTester tester, Duration d) async {
    final g = await tester.startGesture(
      tester.getCenter(find.byKey(const Key('kiosk-logo'))),
    );
    await tester.pump(d);
    await g.up();
    await tester.pump();
    await tester.pump();
  }

  String fieldText(String key) => controllerTextOf(find.byKey(Key(key)));

  setUp(() {
    server = FakeRepository();
    probed = [];
  });

  testWidgets('Android, nothing stored: the setup screen, not the kiosk', (
    tester,
  ) async {
    await pumpApp(tester, device: true);
    expect(find.byType(SetupPage), findsOneWidget);
    expect(find.byType(KioskPage), findsNothing);
    expect(server.calls, isEmpty);
    await finish(tester);
  });

  testWidgets('Android, endpoint stored: the kiosk', (tester) async {
    await pumpApp(
      tester,
      device: true,
      stored: KioskEndpoint.parse('https://shop.example.com', 'MAIN'),
    );
    expect(find.byType(KioskPage), findsOneWidget);
    expect(find.byType(SetupPage), findsNothing);
    await finish(tester);
  });

  testWidgets('the web never shows setup, even with no endpoint', (
    tester,
  ) async {
    await pumpApp(tester, device: false);
    expect(find.byType(KioskPage), findsOneWidget);
    expect(find.byType(SetupPage), findsNothing);
    await finish(tester);
  });

  testWidgets('holding the logo ten seconds opens setup, prefilled, with a '
      'way back', (tester) async {
    await pumpApp(
      tester,
      device: true,
      stored: KioskEndpoint.parse('https://shop.example.com:8443', 'B-7'),
    );
    await holdLogo(tester, const Duration(seconds: 10));
    expect(find.byType(SetupPage), findsOneWidget);
    expect(fieldText('setup-server'), 'https://shop.example.com:8443');
    expect(fieldText('setup-branch'), 'B-7');

    await tester.tap(find.byKey(const Key('setup-back')));
    await tester.pump();
    await tester.pump();
    expect(find.byType(KioskPage), findsOneWidget);
    expect(store.saves, 0);
    await finish(tester);
  });

  testWidgets('9.9 seconds on the logo is not enough', (tester) async {
    await pumpApp(
      tester,
      device: true,
      stored: KioskEndpoint.parse('https://shop.example.com', 'MAIN'),
    );
    await holdLogo(tester, const Duration(milliseconds: 9900));
    await tester.pump(const Duration(seconds: 5));
    expect(find.byType(KioskPage), findsOneWidget);
    expect(find.byType(SetupPage), findsNothing);
    await finish(tester);
  });

  testWidgets('on the web the hold does nothing', (tester) async {
    await pumpApp(
      tester,
      device: false,
      stored: KioskEndpoint.parse('https://shop.example.com', 'MAIN'),
    );
    await holdLogo(tester, const Duration(seconds: 12));
    expect(find.byType(KioskPage), findsOneWidget);
    expect(find.byType(SetupPage), findsNothing);
    await finish(tester);
  });

  testWidgets(
    'first run: type, test, see the branch, save, land on the kiosk',
    (tester) async {
      await pumpApp(tester, device: true);
      expect(find.byKey(const Key('setup-back')), findsNothing);

      await tester.enterText(
        find.byKey(const Key('setup-server')),
        'shop.example.com',
      );
      await tester.enterText(find.byKey(const Key('setup-branch')), 'MAIN');
      await tester.pump();
      // save does nothing before a test
      expect(
        tester
            .widget<ButtonStyleButton>(find.byKey(const Key('setup-save')))
            .onPressed,
        isNull,
      );

      await tester.tap(find.byKey(const Key('setup-test')));
      await tester.pump();
      await tester.pump();
      expect(probed.single.origin, 'https://shop.example.com');
      expect(find.textContaining('Siam Square'), findsOneWidget);

      await tester.tap(find.byKey(const Key('setup-save')));
      await tester.pump();
      await tester.pump();
      await tester.pump();
      expect(find.byType(KioskPage), findsOneWidget);
      expect(
        store.stored,
        KioskEndpoint.parse('https://shop.example.com', 'MAIN'),
      );
      expect(server.calls, contains('rates'));
      await finish(tester);
    },
  );

  testWidgets('editing a field after a test asks for a new test', (
    tester,
  ) async {
    await pumpApp(tester, device: true);
    await tester.enterText(
      find.byKey(const Key('setup-server')),
      'shop.example.com',
    );
    await tester.enterText(find.byKey(const Key('setup-branch')), 'MAIN');
    await tester.tap(find.byKey(const Key('setup-test')));
    await tester.pump();
    await tester.pump();
    await tester.enterText(find.byKey(const Key('setup-branch')), 'OTHER');
    await tester.pump();
    expect(
      tester
          .widget<ButtonStyleButton>(find.byKey(const Key('setup-save')))
          .onPressed,
      isNull,
    );
    await finish(tester);
  });
}

/// The text of the one [TextField] under [finder].
String controllerTextOf(Finder finder) {
  final field = finder.evaluate().single.widget;
  if (field is TextField) return field.controller!.text;
  final inner = find.descendant(of: finder, matching: find.byType(TextField));
  return (inner.evaluate().single.widget as TextField).controller!.text;
}
