import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

// RF-5 (#877, docs/design/blueprint/rates.md §4.10): the kiosk's four
// freshness texts exist in English and Thai, carry their placeholder, and no
// language says "real time" or "live" in them.
// Written before the code by the ticket author (claude/dell-claude-4). Do not
// edit, delete or weaken these tests; add your own in another file.

const assetPath = 'assets/l10n/kiosk.json';
const keys = [
  'rateStale',
  'rateSourceDate',
  'rateNoSourceDate',
  'ratePublishedAt',
];
const placeholders = {'rateSourceDate': '{date}', 'ratePublishedAt': '{at}'};

final forbidden = RegExp(
  r'real\s*-?\s*time|\blive\b|เรียลไทม์',
  caseSensitive: false,
);

Map<String, dynamic> texts() =>
    (jsonDecode(File(assetPath).readAsStringSync())
            as Map<String, dynamic>)['text']
        as Map<String, dynamic>;

void main() {
  for (final language in ['en', 'th']) {
    test('$language has the four texts, each with its placeholder', () {
      final table = texts()[language] as Map<String, dynamic>;
      for (final key in keys) {
        final value = table[key];
        expect(value, isA<String>(), reason: '$language/$key');
        expect((value as String).trim(), isNotEmpty, reason: '$language/$key');
        final p = placeholders[key];
        if (p != null) expect(value, contains(p), reason: '$language/$key');
      }
    });
  }

  test('no language says real time or live in them', () {
    final found = <String>[];
    texts().forEach((language, table) {
      for (final key in keys) {
        final value = (table as Map<String, dynamic>)[key];
        if (value is String && forbidden.hasMatch(value)) {
          found.add('$language/$key: $value');
        }
      }
    });
    expect(found, isEmpty);
  });
}
