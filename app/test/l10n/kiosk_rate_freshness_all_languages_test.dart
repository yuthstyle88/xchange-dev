import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/l10n/kiosk_l10n.dart';

const assetPath = 'assets/l10n/kiosk.json';
const keys = [
  'rateStale',
  'rateSourceDate',
  'rateNoSourceDate',
  'ratePublishedAt',
];
const placeholders = {'rateSourceDate': '{date}', 'ratePublishedAt': '{at}'};

/// Locales omitted per #2164 §3 ("For a locale you cannot translate
/// confidently, leave its four keys out... and list that locale in the PR
/// body with the reason"):
/// - `my`: existing kiosk.json strings do not establish terms for reference rate / published time.
/// - `ja`: bundled subset font NotoSansJP.ttf lacks glyphs for 参考 (参, 考) and 基準 (基, 準), and
///   font assets are outside #2164 file scope.
/// - `ko`: bundled subset font NotoSansKR.ttf lacks glyphs for 기준 (준) and 참조/참고 (참), and
///   font assets are outside #2164 file scope.
const omittedLocales = <String>{'ja', 'ko', 'my'};

final forbidden = RegExp(
  r'real\s*-?\s*time|\blive\b|เรียลไทม์|实时|實時|リアルタイム|실시간|в\s*реальном\s*времени|الوقت\s*الفعلي|temps\s*réel|echtzeit|waktu\s*nyata|masa\s*nyata|thời\s*gian\s*thực',
  caseSensitive: false,
);

Map<String, dynamic> rawAsset() =>
    jsonDecode(File(assetPath).readAsStringSync()) as Map<String, dynamic>;

void main() {
  late Map<String, dynamic> asset;
  late Map<String, dynamic> textTables;
  late List<String> offeredCodes;
  late KioskL10n l10n;

  setUpAll(() {
    asset = rawAsset();
    textTables = asset['text'] as Map<String, dynamic>;
    offeredCodes = [
      for (final entry in asset['langs'] as List<dynamic>)
        (entry as Map<String, dynamic>)['code'] as String,
    ];
    l10n = KioskL10n.fromJson(asset);
  });

  test(
    'row 1: every translated locale has the four keys, non-empty, with placeholders, distinct from notAvailable, and no real-time/live wording',
    () {
      final enTable = textTables['en'] as Map<String, dynamic>;
      final translated = offeredCodes
          .where((c) => !omittedLocales.contains(c))
          .toList();
      expect(translated, isNotEmpty);

      for (final language in translated) {
        final table = textTables[language] as Map<String, dynamic>;
        final notAvailable = (table['notAvailable'] as String?)?.trim();
        for (final key in keys) {
          final value = table[key];
          expect(value, isA<String>(), reason: '$language/$key missing');
          final text = (value as String).trim();
          expect(text, isNotEmpty, reason: '$language/$key empty');
          expect(
            text,
            isNot(equals(notAvailable)),
            reason:
                '$language/$key must not equal that locale\'s notAvailable ($notAvailable)',
          );
          final p = placeholders[key];
          if (p != null) {
            expect(
              text,
              contains(p),
              reason: '$language/$key missing placeholder $p',
            );
          }
          expect(
            forbidden.hasMatch(text),
            isFalse,
            reason: '$language/$key contains forbidden live/real-time wording',
          );
          if (language != 'en') {
            expect(
              text,
              isNot(equals(enTable[key])),
              reason: '$language/$key must be translated, not copied from en',
            );
          }
        }
      }
    },
  );

  test(
    'row 2: locales in the allow-list omit the four keys and fall back to en',
    () {
      for (final code in omittedLocales) {
        expect(offeredCodes, contains(code));
        final table = textTables[code] as Map<String, dynamic>;
        for (final key in keys) {
          expect(
            table.containsKey(key),
            isFalse,
            reason: '$code/$key should be omitted when in omittedLocales',
          );
        }
        final texts = l10n.of(code);
        expect(texts.text('rateStale'), 'Reference out of date');
        expect(
          texts.text('rateSourceDate', {'date': '2026-10-16'}),
          'Reference date 2026-10-16',
        );
        expect(texts.text('rateNoSourceDate'), 'No reference');
        expect(
          texts.text('ratePublishedAt', {'at': '2026-10-16 09:00'}),
          'Published 2026-10-16 09:00',
        );
      }
    },
  );

  test(
    'row 3: ar (right to left) keeps placeholders intact and substitutes them cleanly',
    () {
      final arTable = textTables['ar'] as Map<String, dynamic>;
      expect(arTable['rateSourceDate'], contains('{date}'));
      expect(arTable['ratePublishedAt'], contains('{at}'));

      final ar = l10n.of('ar');
      expect(ar.language.rtl, isTrue);
      expect(
        ar.text('rateSourceDate', {'date': '2026-10-16'}),
        contains('2026-10-16'),
      );
      expect(
        ar.text('rateSourceDate', {'date': '2026-10-16'}),
        isNot(contains('{date}')),
      );
      expect(ar.text('ratePublishedAt', {'at': '09:30'}), contains('09:30'));
      expect(
        ar.text('ratePublishedAt', {'at': '09:30'}),
        isNot(contains('{at}')),
      );
    },
  );

  test(
    'row 4: zh and zh-HK use simplified and traditional scripts respectively, matching each block',
    () {
      final zh = textTables['zh'] as Map<String, dynamic>;
      final zhHk = textTables['zh-HK'] as Map<String, dynamic>;

      // Existing script convention in the block: zh uses 汇率, zh-HK uses 匯率
      expect(zh['rate'], '汇率');
      expect(zhHk['rate'], '匯率');

      for (final key in keys) {
        expect(zh[key], isA<String>(), reason: 'zh/$key');
        expect(zhHk[key], isA<String>(), reason: 'zh-HK/$key');
        expect(
          zh[key],
          isNot(equals(zhHk[key])),
          reason:
              'zh/$key and zh-HK/$key must use simplified vs traditional script',
        );
      }
      expect(zh['rateStale'] as String, contains('基准汇率'));
      expect(zhHk['rateStale'] as String, contains('基準匯率'));
      expect(zh['rateNoSourceDate'] as String, contains('基准汇率'));
      expect(zhHk['rateNoSourceDate'] as String, contains('基準匯率'));
    },
  );
}
