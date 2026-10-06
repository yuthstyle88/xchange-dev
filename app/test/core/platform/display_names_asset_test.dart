// Provided with ticket A2. These tests are the specification: make them pass,
// do not edit, delete or weaken them.
//
// The asset `assets/l10n/display_names.json` is what an installed app shows
// instead of asking a browser. These tests read the real asset.
import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/core/platform/display_names.dart';

const assetPath = 'assets/l10n/display_names.json';

Map<String, dynamic> readJson(String path) {
  final file = File(path);
  expect(file.existsSync(), isTrue, reason: '$path must exist');
  return jsonDecode(file.readAsStringSync()) as Map<String, dynamic>;
}

/// The `lang` tag of each of the kiosk's languages.
List<String> kioskLanguageTags() => [
  for (final l in readJson('assets/l10n/kiosk.json')['langs'] as List)
    '${(l as Map)['lang'] ?? l['code']}',
];

/// `tool/cjk_currency_names.json`: kiosk code → {lang, names}. The four CJK
/// fonts in the app are subsets cut for exactly these names.
Map<String, dynamic> cjkCurrencies() =>
    readJson('tool/cjk_currency_names.json')['currencies']
        as Map<String, dynamic>;

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  group('the recorded table', () {
    late Map<String, dynamic> json;
    late DisplayNames names;
    late Set<String> fontCodes;

    setUp(() {
      json = readJson(assetPath);
      names = BundledDisplayNames.fromJson(json);
      fontCodes = ((cjkCurrencies()['zh'] as Map)['names'] as Map).keys
          .map((k) => '$k')
          .toSet();
    });

    test('says which browser it was recorded with', () {
      expect(json['recordedWith'], isA<String>());
      expect('${json['recordedWith']}'.trim(), isNotEmpty);
    });

    test('has currencies and regions for exactly the 16 kiosk languages, '
        'keyed by language tag', () {
      final tags = kioskLanguageTags().toSet();
      expect(tags.length, 16);
      expect((json['currencies'] as Map).keys.toSet(), tags);
      expect((json['regions'] as Map).keys.toSet(), tags);
      expect(tags, containsAll(['zh-Hans', 'zh-Hant']));
    });

    test('names no currency outside the 159 codes the fonts were cut for, '
        'and English names every one of them', () {
      expect(fontCodes.length, 159);
      final currencies = json['currencies'] as Map<String, dynamic>;
      for (final lang in currencies.keys) {
        final codes = (currencies[lang] as Map).keys.map((k) => '$k').toSet();
        expect(codes.difference(fontCodes), isEmpty, reason: lang);
        expect(codes.length, greaterThan(140), reason: lang);
      }
      for (final code in fontCodes) {
        expect(names.currencyName('en', code), isNotNull, reason: code);
      }
      expect(
        (json['symbols'] as Map).keys
            .map((k) => '$k')
            .toSet()
            .difference(fontCodes),
        isEmpty,
      );
    });

    test('for Chinese, Japanese and Korean it says exactly what '
        'tool/cjk_currency_names.json says — the subset fonts have glyphs '
        'for those names and for no other wording', () {
      var compared = 0;
      for (final entry in cjkCurrencies().values) {
        final lang = '${(entry as Map)['lang']}';
        final recorded = entry['names'] as Map;
        for (final code in recorded.keys) {
          final name = '${recorded[code]}';
          expect(
            names.currencyName(lang, '$code'),
            name == code ? isNull : name,
            reason: '$lang $code',
          );
          compared++;
        }
      }
      expect(compared, 4 * 159);
    });

    test('has the regions of the world in every language', () {
      final regions = json['regions'] as Map<String, dynamic>;
      for (final lang in regions.keys) {
        expect((regions[lang] as Map).length, greaterThan(200), reason: lang);
      }
      expect(names.regionName('en', 'TH'), 'Thailand');
      expect(names.regionName('th', 'TH'), 'ไทย');
    });

    test('every value is a non-empty string', () {
      for (final section in ['currencies', 'regions']) {
        for (final byCode in (json[section] as Map).values) {
          for (final v in (byCode as Map).values) {
            expect(v, isA<String>());
            expect('$v'.trim(), isNotEmpty);
          }
        }
      }
      for (final v in (json['symbols'] as Map).values) {
        expect(v, isA<String>());
        expect('$v'.trim(), isNotEmpty);
      }
    });
  });

  group('parity with the browser', () {
    // Recorded from Chrome by the ticket author with the web implementation's
    // own calls; the file says how and with which versions.
    late Map<String, dynamic> sample;
    setUp(() {
      sample = readJson(
        'test/core/platform/fixtures/browser_display_names_sample.json',
      );
    });

    test('the sample is the one that was recorded', () {
      expect((sample['currencyName'] as List).length, 109);
      expect((sample['regionName'] as List).length, 92);
      expect((sample['currencySymbol'] as List).length, 13);
    });

    test('currency names: the table answers what the browser answered', () {
      final names = BundledDisplayNames.fromJson(readJson(assetPath));
      for (final row in sample['currencyName'] as List) {
        final [lang, code, want] = row as List;
        expect(
          names.currencyName('$lang', '$code'),
          want,
          reason: 'currencyName($lang, $code)',
        );
      }
    });

    test('region names: the table answers what the browser answered', () {
      final names = BundledDisplayNames.fromJson(readJson(assetPath));
      for (final row in sample['regionName'] as List) {
        final [lang, code, want] = row as List;
        expect(
          names.regionName('$lang', '$code'),
          want,
          reason: 'regionName($lang, $code)',
        );
      }
    });

    test('currency symbols: the table answers what the browser answered', () {
      final names = BundledDisplayNames.fromJson(readJson(assetPath));
      for (final row in sample['currencySymbol'] as List) {
        final [code, want] = row as List;
        expect(
          names.currencySymbol('$code'),
          want,
          reason: 'currencySymbol($code)',
        );
      }
    });
  });

  group('loading', () {
    test('the asset is declared in pubspec.yaml', () {
      final pubspec = File('pubspec.yaml').readAsStringSync();
      expect(
        RegExp(
          r'^\s+-\s+assets/l10n/display_names\.json\s*$',
          multiLine: true,
        ).hasMatch(pubspec),
        isTrue,
      );
    });

    test('BundledDisplayNames.load reads it from the app bundle', () async {
      expect(BundledDisplayNames.assetPath, assetPath);
      final names = await BundledDisplayNames.load();
      expect(names.currencyName('th', 'USD'), 'ดอลลาร์สหรัฐ');
      expect(names.regionName('ja', 'TH'), 'タイ');
      expect(names.currencySymbol('THB'), '฿');
    });

    test('loadDisplayNames is what main awaits before the first frame: on a '
        'device it is the loaded table', () async {
      final names = await loadDisplayNames();
      expect(names, isA<BundledDisplayNames>());
      expect(names.currencyName('th', 'USD'), 'ดอลลาร์สหรัฐ');
    });

    test('main loads the names and hands them to the app', () {
      final main = File('lib/main.dart').readAsStringSync();
      expect(main, contains('loadDisplayNames('));
      expect(main, contains('displayNamesProvider.overrideWith'));
    });
  });

  group('the recorder', () {
    test('is in the repository and asks the browser the way the web build '
        'does', () {
      final file = File('tool/record_display_names.mjs');
      expect(file.existsSync(), isTrue);
      final src = file.readAsStringSync();
      expect(src, contains('Intl.DisplayNames'));
      expect(src, contains('narrowSymbol'));
      expect(src, contains('display_names.json'));
      expect(src, contains('cjk_currency_names.json'));
    });
  });
}
