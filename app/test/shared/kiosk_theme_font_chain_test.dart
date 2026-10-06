import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/app/app.dart';
import 'package:xchange_kiosk/features/kiosk/app/kiosk_providers.dart';
import 'package:xchange_kiosk/features/kiosk/domain/entities/kiosk_models.dart';
import 'package:xchange_kiosk/features/kiosk/domain/repository/kiosk_repository.dart';
import 'package:xchange_kiosk/l10n/kiosk_l10n.dart';
import 'package:xchange_kiosk/shared/theme/theme.dart';

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
  }) async => throw UnimplementedError();

  @override
  Future<RequestIdentity> confirm(String id) async =>
      throw UnimplementedError();

  @override
  Future<RequestIdentity> status(String id) async => throw UnimplementedError();

  @override
  Future<ReceiptModel> customerReceipt(String id) async =>
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
  const allNineFamilies = [
    'NotoSansThai',
    'NotoSansArabic',
    'NotoSansDevanagari',
    'NotoSansLao',
    'NotoSansMyanmar',
    'NotoSansSC',
    'NotoSansTC',
    'NotoSansJP',
    'NotoSansKR',
  ];

  test(
    'for every language in assets/l10n/kiosk.json the chain has all nine families',
    () {
      final file = _findFile('assets/l10n/kiosk.json');
      final json = jsonDecode(file.readAsStringSync()) as Map<String, dynamic>;
      final langs = json['langs'] as List<dynamic>;

      for (final l in langs) {
        final code = '${l['code']}';
        final fallbacks = fontFallbacksForLanguage(code);

        expect(
          fallbacks.length,
          equals(9),
          reason: 'Language "$code" must have exactly 9 fallback font families',
        );
        expect(
          fallbacks.toSet().length,
          equals(9),
          reason: 'Language "$code" fallback font families must be unique',
        );
        for (final family in allNineFamilies) {
          expect(
            fallbacks,
            contains(family),
            reason:
                'Language "$code" must include $family in its fallback chain',
          );
        }

        final theme = kioskTheme(code);
        final themeFallbacks =
            theme.textTheme.bodyMedium?.fontFamilyFallback ?? [];
        expect(
          themeFallbacks.length,
          equals(9),
          reason:
              'Theme for "$code" must have all 9 font families in textTheme fallback',
        );
      }
    },
  );

  test(
    'ja starts with NotoSansJP, ko with NotoSansKR, zh with NotoSansSC, zh-HK with NotoSansTC',
    () {
      expect(fontFallbacksForLanguage('ja').first, equals('NotoSansJP'));
      expect(fontFallbacksForLanguage('ko').first, equals('NotoSansKR'));
      expect(fontFallbacksForLanguage('zh').first, equals('NotoSansSC'));
      expect(fontFallbacksForLanguage('zh-HK').first, equals('NotoSansTC'));

      expect(
        kioskTheme('ja').textTheme.bodyMedium?.fontFamilyFallback?.first,
        equals('NotoSansJP'),
      );
      expect(
        kioskTheme('ko').textTheme.bodyMedium?.fontFamilyFallback?.first,
        equals('NotoSansKR'),
      );
      expect(
        kioskTheme('zh').textTheme.bodyMedium?.fontFamilyFallback?.first,
        equals('NotoSansSC'),
      );
      expect(
        kioskTheme('zh-HK').textTheme.bodyMedium?.fontFamilyFallback?.first,
        equals('NotoSansTC'),
      );
    },
  );

  test(
    'for th (a non-CJK language) the chain is the five non-CJK fonts then SC, TC, JP, KR',
    () {
      const expectedThChain = [
        'NotoSansThai',
        'NotoSansArabic',
        'NotoSansDevanagari',
        'NotoSansLao',
        'NotoSansMyanmar',
        'NotoSansSC',
        'NotoSansTC',
        'NotoSansJP',
        'NotoSansKR',
      ];

      expect(fontFallbacksForLanguage('th'), equals(expectedThChain));
      expect(
        kioskTheme('th').textTheme.bodyMedium?.fontFamilyFallback,
        equals(expectedThChain),
      );
    },
  );

  testWidgets(
    'pump KioskApp with the controller at th, call setLanguage(\'ja\'), and the MaterialApp theme\'s fontFamilyFallback now starts with NotoSansJP',
    (tester) async {
      final file = _findFile('assets/l10n/kiosk.json');
      final l10n = KioskL10n.fromJson(
        jsonDecode(file.readAsStringSync()) as Map<String, dynamic>,
      );

      final container = ProviderContainer(
        overrides: [
          kioskL10nProvider.overrideWithValue(l10n),
          kioskRepositoryProvider.overrideWithValue(_FakeRepository()),
        ],
      );
      addTearDown(container.dispose);

      // Explicitly set controller to 'th'
      container.read(kioskControllerProvider.notifier).setLanguage('th');

      await tester.pumpWidget(
        UncontrolledProviderScope(
          container: container,
          child: const KioskApp(),
        ),
      );
      await tester.pumpAndSettle();

      // Check initial theme for 'th'
      var materialApp = tester.widget<MaterialApp>(find.byType(MaterialApp));
      expect(
        materialApp.theme?.textTheme.bodyMedium?.fontFamilyFallback?.first,
        equals('NotoSansThai'),
        reason: 'Initial theme for th should start with NotoSansThai',
      );

      // Change language to 'ja'
      container.read(kioskControllerProvider.notifier).setLanguage('ja');
      await tester.pumpAndSettle();

      // Verify MaterialApp theme updated dynamically to start with NotoSansJP
      materialApp = tester.widget<MaterialApp>(find.byType(MaterialApp));
      expect(
        materialApp.theme?.textTheme.bodyMedium?.fontFamilyFallback?.first,
        equals('NotoSansJP'),
        reason:
            'After changing language to ja, MaterialApp theme fontFamilyFallback must start with NotoSansJP',
      );
    },
  );
}
