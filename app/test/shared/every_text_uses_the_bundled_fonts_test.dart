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

void main() {
  test('pubspec.yaml declares a family Roboto backed by NotoSans', () {
    final pubspecFile = _findFile('pubspec.yaml');
    final content = pubspecFile.readAsStringSync();

    // Verify family Roboto declaration
    final robotoFamilyPattern = RegExp(
      r'-\s+family:\s+Roboto\s*\n\s+fonts:\s*\n\s+-\s+asset:\s+(assets/fonts/NotoSans\.ttf)',
    );
    expect(
      robotoFamilyPattern.hasMatch(content),
      isTrue,
      reason:
          'pubspec.yaml must declare a family named Roboto pointing to assets/fonts/NotoSans.ttf',
    );
  });

  testWidgets(
    'every RichText on select page and quote page has fontFamilyFallback containing NotoSansThai',
    (tester) async {
      tester.view.physicalSize = const Size(1280, 1600);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);

      final l10nFile = _findFile('assets/l10n/kiosk.json');
      final l10n = KioskL10n.fromJson(
        jsonDecode(l10nFile.readAsStringSync()) as Map<String, dynamic>,
      );

      final container = ProviderContainer(
        overrides: [
          kioskL10nProvider.overrideWithValue(l10n),
          kioskRepositoryProvider.overrideWithValue(_FakeRepository()),
          deviceLanguagesProvider.overrideWithValue(['th']),
        ],
      );

      // Explicitly set language to Thai and load rates
      final c = container.read(kioskControllerProvider.notifier);
      c.setLanguage('th');
      await c.start();
      c
        ..setCurrency('USD')
        ..setAmount('100')
        ..setPaymentMethod('Cash');

      await tester.pumpWidget(
        UncontrolledProviderScope(
          container: container,
          child: const KioskApp(),
        ),
      );
      await tester.pumpAndSettle();

      expect(find.byType(SelectPage), findsOneWidget);

      // Assert on payment-method dropdown text by name and key
      final dropdownFinder = find.byKey(const ValueKey('payment-BUY-Cash'));
      expect(dropdownFinder, findsOneWidget);
      final dropdownText = find.descendant(
        of: dropdownFinder,
        matching: find.byType(Text),
      );
      expect(dropdownText, findsOneWidget);
      final dropdownRichText = tester.widget<RichText>(
        find.descendant(of: dropdownText, matching: find.byType(RichText)),
      );
      expect(
        dropdownRichText.text.style?.fontFamilyFallback,
        contains('NotoSansThai'),
        reason:
            'Payment-method dropdown text resolved style must contain NotoSansThai',
      );

      // Assert every RichText on SelectPage has fontFamilyFallback containing NotoSansThai
      final allSelectRichTexts = tester.widgetList<RichText>(
        find.byType(RichText),
      );
      expect(allSelectRichTexts, isNotEmpty);
      for (final richText in allSelectRichTexts) {
        // Skip icon glyphs (e.g. MaterialIcons)
        if (richText.text.style?.fontFamily == 'MaterialIcons') continue;
        final style = richText.text.style;
        expect(
          style?.fontFamilyFallback,
          contains('NotoSansThai'),
          reason:
              'Every text RichText on select page must have fontFamilyFallback containing NotoSansThai. Failed text: "${richText.text.toPlainText()}" with style $style',
        );
      }

      // Tap review button to move to QuotePage
      final reviewButtonFinder = find.byKey(const Key('review'));
      expect(reviewButtonFinder, findsOneWidget);
      await tester.tap(reviewButtonFinder);
      await tester.pumpAndSettle();

      expect(find.byType(QuotePage), findsOneWidget);

      // Assert on confirm button label by name and key
      final confirmButtonFinder = find.byKey(const Key('confirm'));
      expect(confirmButtonFinder, findsOneWidget);
      final confirmRichTextFinder = find.descendant(
        of: confirmButtonFinder,
        matching: find.byType(RichText),
      );
      expect(confirmRichTextFinder, findsOneWidget);
      final confirmRichText = tester.widget<RichText>(confirmRichTextFinder);
      expect(
        confirmRichText.text.style?.fontFamilyFallback,
        contains('NotoSansThai'),
        reason:
            'Confirm button label resolved style must contain NotoSansThai: $confirmRichText',
      );

      // Assert on back button label (SecondaryButton) as well
      final backButtonFinder = find.byKey(const Key('back'));
      expect(backButtonFinder, findsOneWidget);
      final backRichTextFinder = find.descendant(
        of: backButtonFinder,
        matching: find.byType(RichText),
      );
      expect(backRichTextFinder, findsOneWidget);
      final backRichText = tester.widget<RichText>(backRichTextFinder);
      expect(
        backRichText.text.style?.fontFamilyFallback,
        contains('NotoSansThai'),
        reason:
            'Back button label resolved style must contain NotoSansThai: $backRichText',
      );

      // Assert every RichText on QuotePage has fontFamilyFallback containing NotoSansThai
      final allQuoteRichTexts = tester.widgetList<RichText>(
        find.byType(RichText),
      );
      expect(allQuoteRichTexts, isNotEmpty);
      for (final richText in allQuoteRichTexts) {
        if (richText.text.style?.fontFamily == 'MaterialIcons') continue;
        final style = richText.text.style;
        expect(
          style?.fontFamilyFallback,
          contains('NotoSansThai'),
          reason:
              'Every text RichText on quote page must have fontFamilyFallback containing NotoSansThai. Failed text: "${richText.text.toPlainText()}" with style $style',
        );
      }

      container.dispose();
    },
  );
}
