import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

void main() {
  const assetPath = 'assets/l10n/queue_machine.json';

  test(
    'queue_machine.json has th and en only, matching keys, non-empty, and Thai text',
    () {
      final file = File(assetPath);
      expect(file.existsSync(), isTrue, reason: '$assetPath must exist');

      final json = jsonDecode(file.readAsStringSync()) as Map<String, dynamic>;
      expect(json.keys.toSet(), equals({'en', 'th'}));

      final en = Map<String, String>.from(json['en'] as Map<String, dynamic>);
      final th = Map<String, String>.from(json['th'] as Map<String, dynamic>);

      expect(
        en.keys.toSet(),
        equals(th.keys.toSet()),
        reason: 'keys must match exactly',
      );
      expect(en.isNotEmpty, isTrue, reason: 'must have keys');

      final thaiRegExp = RegExp(r'[\u0E00-\u0E7F]');

      for (final key in en.keys) {
        expect(en[key], isNotEmpty, reason: 'en[$key] must not be empty');
        expect(th[key], isNotEmpty, reason: 'th[$key] must not be empty');
        expect(
          thaiRegExp.hasMatch(th[key]!),
          isTrue,
          reason: 'th[$key] must contain Thai characters',
        );
      }
    },
  );
}
