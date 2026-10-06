// Provided with ticket A2. These tests are the specification: make them pass,
// do not edit, delete or weaken them.
//
// On the Dart VM the conditional import picks the device implementation
// (`BundledDisplayNames`), the one an installed app uses. The browser
// implementation runs only in a browser; what it answers is pinned by the
// recorded sample in display_names_asset_test.dart.
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/core/platform/display_names.dart';

/// A table in the shape of `assets/l10n/display_names.json`, small enough to
/// read: the outer keys of `currencies` and `regions` are BCP 47 tags (the
/// `lang` of a kiosk language, e.g. `zh-Hant`), not kiosk codes (`zh-HK`).
Map<String, dynamic> table() => {
  'recordedWith': 'a browser',
  'symbols': {'USD': r'$', 'THB': '฿'},
  'currencies': {
    'en': {'USD': 'US Dollar', 'THB': 'Thai Baht', 'JPY': 'Japanese Yen'},
    'th': {'USD': 'ดอลลาร์สหรัฐ', 'THB': 'บาท', 'XTS': 'XTS'},
    'zh-Hant': {'USD': '美元'},
  },
  'regions': {
    'en': {'TH': 'Thailand', 'JP': 'Japan'},
    'th': {'TH': 'ไทย', 'ZZ': 'ZZ'},
  },
};

void main() {
  late DisplayNames names;
  setUp(() => names = BundledDisplayNames.fromJson(table()));

  group('currency name', () {
    test('is looked up by language tag and code', () {
      expect(names.currencyName('th', 'USD'), 'ดอลลาร์สหรัฐ');
      expect(names.currencyName('en', 'USD'), 'US Dollar');
      expect(names.currencyName('zh-Hant', 'USD'), '美元');
    });

    test('a language the table lacks gets the English name, as the browser '
        'does when asked for [lang, en]', () {
      expect(names.currencyName('tlh', 'JPY'), 'Japanese Yen');
      // The kiosk code of Traditional Chinese is not its language tag.
      expect(names.currencyName('zh-HK', 'USD'), 'US Dollar');
    });

    test('a code the language lacks is null — the English name of that code '
        'is NOT borrowed', () {
      expect(names.currencyName('th', 'JPY'), isNull);
      expect(names.currencyName('zh-Hant', 'THB'), isNull);
    });

    test('an unknown code is null in every language', () {
      for (final lang in ['th', 'en', 'tlh']) {
        expect(names.currencyName(lang, 'XXZ'), isNull, reason: lang);
        expect(names.currencyName(lang, ''), isNull, reason: lang);
        expect(names.currencyName(lang, 'DOLLAR'), isNull, reason: lang);
      }
    });

    test('a recorded "name" equal to the code is no name', () {
      expect(names.currencyName('th', 'XTS'), isNull);
    });

    test('the code is matched without regard to case, as the browser does', () {
      expect(names.currencyName('th', 'usd'), 'ดอลลาร์สหรัฐ');
      expect(names.currencyName('en', 'Thb'), 'Thai Baht');
    });
  });

  group('region name', () {
    test('is looked up by language tag and code', () {
      expect(names.regionName('th', 'TH'), 'ไทย');
      expect(names.regionName('en', 'JP'), 'Japan');
    });

    test('a language the table lacks gets the English name', () {
      expect(names.regionName('tlh', 'TH'), 'Thailand');
    });

    test('a code the language lacks, an unknown code and a "name" equal to '
        'the code are null', () {
      expect(names.regionName('th', 'JP'), isNull);
      expect(names.regionName('en', 'QN'), isNull);
      expect(names.regionName('en', ''), isNull);
      expect(names.regionName('th', 'ZZ'), isNull);
    });

    test('the code is matched exactly — a browser answers nothing for a '
        'lower-case region', () {
      expect(names.regionName('th', 'th'), isNull);
      expect(names.regionName('en', 'jp'), isNull);
    });
  });

  group('currency symbol', () {
    test('is the recorded symbol', () {
      expect(names.currencySymbol('USD'), r'$');
      expect(names.currencySymbol('THB'), '฿');
    });

    test('is the code itself when the table has none', () {
      expect(names.currencySymbol('CHF'), 'CHF');
      expect(names.currencySymbol('XXZ'), 'XXZ');
      expect(names.currencySymbol(''), '');
    });

    test('the code is matched without regard to case', () {
      expect(names.currencySymbol('usd'), r'$');
    });
  });

  group('a table with parts missing', () {
    test('answers "no name" instead of throwing', () {
      for (final json in <Map<String, dynamic>>[
        {},
        {'symbols': <String, dynamic>{}},
        {
          'currencies': {'th': <String, dynamic>{}},
        },
        {'recordedWith': 'x', 'regions': <String, dynamic>{}},
      ]) {
        final n = BundledDisplayNames.fromJson(json);
        expect(n.currencySymbol('USD'), 'USD', reason: '$json');
        expect(n.currencyName('th', 'USD'), isNull, reason: '$json');
        expect(n.regionName('th', 'TH'), isNull, reason: '$json');
      }
    });

    test('the empty table knows nothing', () {
      const n = BundledDisplayNames.empty();
      expect(n.currencySymbol('USD'), 'USD');
      expect(n.currencyName('en', 'USD'), isNull);
      expect(n.regionName('en', 'TH'), isNull);
    });
  });

  group('provider', () {
    test('without an override the device build has names that know nothing '
        'yet — synchronous, no asset needed, so a test that does not care '
        'needs no override', () {
      final c = ProviderContainer();
      addTearDown(c.dispose);
      final n = c.read(displayNamesProvider);
      expect(n, isA<BundledDisplayNames>());
      expect(n.currencySymbol('USD'), 'USD');
      expect(n.currencyName('en', 'USD'), isNull);
      expect(createDisplayNames(), isA<BundledDisplayNames>());
    });

    test('can be overridden', () {
      final c = ProviderContainer(
        overrides: [displayNamesProvider.overrideWithValue(names)],
      );
      addTearDown(c.dispose);
      expect(c.read(displayNamesProvider), same(names));
      expect(c.read(displayNamesProvider).currencyName('th', 'THB'), 'บาท');
    });
  });
}
