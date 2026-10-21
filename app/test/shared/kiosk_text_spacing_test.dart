import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/app/app.dart';
import 'package:xchange_kiosk/features/kiosk/app/kiosk_providers.dart';
import 'package:xchange_kiosk/features/kiosk/domain/entities/kiosk_models.dart';
import 'package:xchange_kiosk/features/kiosk/domain/repository/kiosk_repository.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/pages/quote_page.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/pages/select_page.dart';
import 'package:xchange_kiosk/l10n/kiosk_l10n.dart';

class _FakeRepository implements KioskRepository {
  @override
  Future<RatesModel> rates() async => RatesModel.fromJson({
    'branchId': 'MAIN',
    'branchName': 'XC-B',
    'open': true,
    'rates': [
      {'code': 'USD', 'nameEn': 'US Dollar', 'buy': 32.1, 'sell': 32.6},
    ],
  });

  @override
  Future<QuoteModel> quote({
    required ExchangeType type,
    required String currency,
    required String amount,
    required String paymentMethod,
  }) async => QuoteModel.fromJson({
    'id': 'quote-1',
    'queue': 'K-001',
    'orderNo': 'ORD-001',
    'status': 'QUOTED',
    'paymentMethod': paymentMethod,
    'type': type.wire,
    'currency': currency,
    'rate': 32.1,
    'foreign': 100.0,
    'thb': 3210.0,
  });

  @override
  Future<RequestIdentity> confirm(String id) async =>
      throw UnimplementedError();

  @override
  Future<RequestIdentity> status(String id) async => throw UnimplementedError();

  @override
  Future<ReceiptModel> customerReceipt(String id) async =>
      throw UnimplementedError();

  @override
  Future<RequestIdentity> customerCancel(String id, String cancelToken) async =>
      throw UnimplementedError();
}

File _findFile(String relativePath) {
  final candidates = [relativePath, 'app/$relativePath', '../$relativePath'];
  for (final path in candidates) {
    final file = File(path);
    if (file.existsSync()) return file;
  }
  throw StateError('Could not find file: $relativePath');
}

Future<ProviderContainer> _pumpKiosk(
  WidgetTester tester, {
  String language = 'th',
  bool navigateToQuote = false,
}) async {
  tester.view.physicalSize = const Size(1280, 1600);
  tester.view.devicePixelRatio = 1.0;

  final l10nFile = _findFile('assets/l10n/kiosk.json');
  final l10n = KioskL10n.fromJson(
    jsonDecode(l10nFile.readAsStringSync()) as Map<String, dynamic>,
  );

  final container = ProviderContainer(
    overrides: [
      kioskL10nProvider.overrideWithValue(l10n),
      kioskRepositoryProvider.overrideWithValue(_FakeRepository()),
      deviceLanguagesProvider.overrideWithValue([language]),
    ],
  );

  final c = container.read(kioskControllerProvider.notifier);
  c.setLanguage(language);
  await c.start();
  c
    ..setCurrency('USD')
    ..setAmount('100')
    ..setPaymentMethod('Cash');

  await tester.pumpWidget(
    UncontrolledProviderScope(container: container, child: const KioskApp()),
  );
  await tester.pumpAndSettle();

  if (navigateToQuote) {
    final reviewButton = find.byKey(const Key('review'));
    expect(reviewButton, findsOneWidget);
    await tester.tap(reviewButton);
    await tester.pumpAndSettle();
  }

  return container;
}

void main() {
  testWidgets(
    'payment-method dropdown value: fontSize 20, no letterSpacing, no height, Thai font fallback',
    (tester) async {
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);

      final container = await _pumpKiosk(tester, language: 'th');
      try {
        expect(find.byType(SelectPage), findsOneWidget);

        final dropdownFinder = find.byKey(const ValueKey('payment-BUY-Cash'));
        expect(dropdownFinder, findsOneWidget);

        final dropdownRichText = tester.widget<RichText>(
          find
              .descendant(of: dropdownFinder, matching: find.byType(RichText))
              .first,
        );
        final style = dropdownRichText.text.style;

        expect(style?.fontSize, 20);
        expect(style?.letterSpacing, isNull);
        expect(style?.height, isNull);
        expect(style?.fontFamilyFallback, contains('NotoSansThai'));
      } finally {
        container.dispose();
      }
    },
  );

  testWidgets(
    'PrimaryButton label: fontSize 24, fontWeight w700, letterSpacing null or 0.0, no height, Thai font fallback',
    (tester) async {
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);

      final container = await _pumpKiosk(tester, language: 'th');
      try {
        final reviewButtonFinder = find.byKey(const Key('review'));
        expect(reviewButtonFinder, findsOneWidget);

        final richText = tester.widget<RichText>(
          find
              .descendant(
                of: reviewButtonFinder,
                matching: find.byType(RichText),
              )
              .first,
        );
        final style = richText.text.style;

        expect(style?.fontSize, 24);
        expect(style?.fontWeight, FontWeight.w700);
        expect(style?.letterSpacing, anyOf(isNull, 0.0));
        expect(style?.height, isNull);
        expect(style?.fontFamilyFallback, contains('NotoSansThai'));
      } finally {
        container.dispose();
      }
    },
  );

  testWidgets(
    'SecondaryButton label: fontSize 22, fontWeight w700, letterSpacing null or 0.0, no height, Thai font fallback',
    (tester) async {
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);

      final container = await _pumpKiosk(
        tester,
        language: 'th',
        navigateToQuote: true,
      );
      try {
        expect(find.byType(QuotePage), findsOneWidget);

        final backButtonFinder = find.byKey(const Key('back'));
        expect(backButtonFinder, findsOneWidget);

        final richText = tester.widget<RichText>(
          find
              .descendant(of: backButtonFinder, matching: find.byType(RichText))
              .first,
        );
        final style = richText.text.style;

        expect(style?.fontSize, 22);
        expect(style?.fontWeight, FontWeight.w700);
        expect(style?.letterSpacing, anyOf(isNull, 0.0));
        expect(style?.height, isNull);
        expect(style?.fontFamilyFallback, contains('NotoSansThai'));
      } finally {
        container.dispose();
      }
    },
  );

  testWidgets(
    'payment-method dropdown value follows kiosk language (ja starts with NotoSansJP)',
    (tester) async {
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);

      final container = await _pumpKiosk(tester, language: 'ja');
      try {
        expect(find.byType(SelectPage), findsOneWidget);

        final dropdownFinder = find.byKey(const ValueKey('payment-BUY-Cash'));
        expect(dropdownFinder, findsOneWidget);

        final dropdownRichText = tester.widget<RichText>(
          find
              .descendant(of: dropdownFinder, matching: find.byType(RichText))
              .first,
        );
        final style = dropdownRichText.text.style;

        expect(style?.fontFamilyFallback, isNotNull);
        expect(style?.fontFamilyFallback?.first, 'NotoSansJP');
        expect(style?.letterSpacing, isNull);
        expect(style?.height, isNull);
      } finally {
        container.dispose();
      }
    },
  );
}
