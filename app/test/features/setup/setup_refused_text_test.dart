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
import 'package:xchange_kiosk/features/kiosk/domain/kiosk_failure.dart';
import 'package:xchange_kiosk/features/setup/app/setup_providers.dart';
import 'package:xchange_kiosk/features/setup/l10n/setup_l10n.dart';

import '../kiosk/kiosk_controller_test.dart' show FakeRepository, l10n;
import 'setup_controller_test.dart' show MemoryStore;

SetupL10n _loadTexts() => SetupL10n.fromJson(
  jsonDecode(File('assets/l10n/setup.json').readAsStringSync())
      as Map<String, dynamic>,
);

void main() {
  late ProviderContainer container;
  late MemoryStore store;
  late FakeRepository server;
  late Future<RatesModel> Function(KioskEndpoint) answer;

  Future<void> pumpSetup(
    WidgetTester tester, {
    required List<String> deviceLanguages,
  }) async {
    tester.view.physicalSize = const Size(1280, 900);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);
    store = MemoryStore();
    final texts = _loadTexts();
    container = ProviderContainer(
      overrides: [
        endpointStoreProvider.overrideWithValue(store),
        initialKioskEndpointProvider.overrideWithValue(null),
        setupAvailableProvider.overrideWithValue(true),
        allowInsecureServerProvider.overrideWithValue(false),
        setupL10nProvider.overrideWith((ref) async => texts),
        setupProbeProvider.overrideWithValue((e) => answer(e)),
        kioskRepositoryProvider.overrideWithValue(server),
        kioskL10nProvider.overrideWithValue(l10n()),
        deviceLanguagesProvider.overrideWithValue(deviceLanguages),
      ],
    );
    await tester.pumpWidget(
      UncontrolledProviderScope(container: container, child: const KioskApp()),
    );
    await tester.pump();
    await tester.pump();
  }

  Future<void> finish(WidgetTester tester) async {
    await tester.pumpWidget(const SizedBox());
    container.dispose();
  }

  Future<void> submitTest(
    WidgetTester tester, {
    String serverInput = 'shop.example.com',
    String branchInput = 'MAIN',
  }) async {
    await tester.enterText(find.byKey(const Key('setup-server')), serverInput);
    await tester.enterText(find.byKey(const Key('setup-branch')), branchInput);
    await tester.tap(find.byKey(const Key('setup-test')));
    await tester.pump();
    await tester.pump();
  }

  setUp(() {
    server = FakeRepository();
    answer = (_) async => throw const KioskFailure('Request rejected.');
  });

  testWidgets('the test is refused with Request rejected. on th shows the th '
      'serverRefused text, then Request rejected. as the detail', (
    tester,
  ) async {
    await pumpSetup(tester, deviceLanguages: const ['th-TH']);
    await submitTest(tester);

    final errorFinder = find.byKey(const Key('setup-error'));
    final detailFinder = find.byKey(const Key('setup-error-detail'));
    expect(errorFinder, findsOneWidget);
    expect(detailFinder, findsOneWidget);
    expect(
      tester.widget<Text>(errorFinder).data,
      'เซิร์ฟเวอร์ปฏิเสธการทดสอบการเชื่อมต่อ',
    );
    expect(tester.widget<Text>(detailFinder).data, 'Request rejected.');
    expect(
      tester.getTopLeft(errorFinder).dy,
      lessThan(tester.getTopLeft(detailFinder).dy),
    );
    await finish(tester);
  });

  testWidgets('the test is refused with Request rejected. on en shows the en '
      'serverRefused text, then Request rejected. as the detail', (
    tester,
  ) async {
    await pumpSetup(tester, deviceLanguages: const ['en-US']);
    await submitTest(tester);

    final errorFinder = find.byKey(const Key('setup-error'));
    final detailFinder = find.byKey(const Key('setup-error-detail'));
    expect(errorFinder, findsOneWidget);
    expect(detailFinder, findsOneWidget);
    expect(
      tester.widget<Text>(errorFinder).data,
      'The server refused the connection test.',
    );
    expect(tester.widget<Text>(detailFinder).data, 'Request rejected.');
    expect(
      tester.getTopLeft(errorFinder).dy,
      lessThan(tester.getTopLeft(detailFinder).dy),
    );
    await finish(tester);
  });

  testWidgets(
    'unreachable (no server message) on th shows unreachable in th with no '
    'detail line',
    (tester) async {
      answer = (_) async => throw const KioskFailure('');
      await pumpSetup(tester, deviceLanguages: const ['th']);
      await submitTest(tester);

      final errorFinder = find.byKey(const Key('setup-error'));
      expect(errorFinder, findsOneWidget);
      expect(
        tester.widget<Text>(errorFinder).data,
        'เซิร์ฟเวอร์ไม่ตอบ ตรวจที่อยู่และเครือข่ายอีกครั้ง',
      );
      expect(find.byKey(const Key('setup-error-detail')), findsNothing);

      // When the server answers with a refusal instead, the th serverRefused
      // headline and detail line appear.
      answer = (_) async => throw const KioskFailure('Request rejected.');
      await tester.tap(find.byKey(const Key('setup-test')));
      await tester.pump();
      await tester.pump();
      expect(
        tester.widget<Text>(errorFinder).data,
        'เซิร์ฟเวอร์ปฏิเสธการทดสอบการเชื่อมต่อ',
      );
      expect(find.byKey(const Key('setup-error-detail')), findsOneWidget);
      await finish(tester);
    },
  );

  testWidgets(
    'an invalid server or branch format shows the format error with no detail '
    'line',
    (tester) async {
      await pumpSetup(tester, deviceLanguages: const ['en']);

      await submitTest(tester, serverInput: '', branchInput: 'MAIN');
      final errorFinder = find.byKey(const Key('setup-error'));
      expect(errorFinder, findsOneWidget);
      expect(
        tester.widget<Text>(errorFinder).data,
        'Enter the server address.',
      );
      expect(find.byKey(const Key('setup-error-detail')), findsNothing);

      await submitTest(
        tester,
        serverInput: 'shop.example.com',
        branchInput: 'BAD BRANCH',
      );
      expect(
        tester.widget<Text>(errorFinder).data,
        'A branch code has no spaces and is at most 80 characters.',
      );
      expect(find.byKey(const Key('setup-error-detail')), findsNothing);

      // Once the format is valid and the server refuses, serverRefused + detail
      // are rendered.
      await submitTest(
        tester,
        serverInput: 'shop.example.com',
        branchInput: 'MAIN',
      );
      expect(
        tester.widget<Text>(errorFinder).data,
        'The server refused the connection test.',
      );
      expect(find.byKey(const Key('setup-error-detail')), findsOneWidget);
      await finish(tester);
    },
  );
}
