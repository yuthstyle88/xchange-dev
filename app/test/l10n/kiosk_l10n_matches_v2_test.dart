import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/l10n/kiosk_l10n.dart';

// The kiosk's texts are the v2 kiosk's, moved 1:1 (#7). This test keeps that
// true: every sentence of assets/l10n/kiosk.json, in every language, equals
// the checked-in copy of the v2 texts in test/l10n/fixtures/.
//
// An intended wording change must update the asset and the fixture together,
// in one pull request that says so.
//
// The fixture is read from the repository, never from a developer's machine.
// When it is missing this test fails; it does not skip.

const fixturePath = 'test/l10n/fixtures/v2_kiosk_texts.json';

/// The file at [path] as a JSON object. A missing file is a failure.
Map<String, dynamic> readJson(String path) {
  final file = File(path);
  if (!file.existsSync()) {
    fail('$path is missing: the v2 texts cannot be compared without it');
  }
  return jsonDecode(file.readAsStringSync()) as Map<String, dynamic>;
}

/// Every place where [actual] and [expected] differ, both ways, each named
/// as `<path>/<key>`: a key only one side has, or a value that is not equal.
/// Two maps under the same key are compared key by key.
List<String> differences(
  Map<String, dynamic> actual,
  Map<String, dynamic> expected, [
  String path = '',
]) {
  final found = <String>[];
  final keys = {...actual.keys, ...expected.keys}.toList()..sort();
  for (final key in keys) {
    final at = path.isEmpty ? key : '$path/$key';
    if (!expected.containsKey(key)) {
      found.add('$at: in the asset, not in the v2 fixture');
    } else if (!actual.containsKey(key)) {
      found.add('$at: in the v2 fixture, not in the asset');
    } else {
      final a = actual[key];
      final e = expected[key];
      if (a is Map<String, dynamic> && e is Map<String, dynamic>) {
        found.addAll(differences(a, e, at));
      } else if (a != e) {
        found.add('$at: asset ${jsonEncode(a)}, v2 ${jsonEncode(e)}');
      }
    }
  }
  return found;
}

void main() {
  group('the comparison itself', () {
    final same = {
      'th': {'a': 'ก', 'b': 'ข'},
      'lo': {'a': 'ກ'},
    };

    test('equal tables have no difference', () {
      expect(differences(same, jsonDecode(jsonEncode(same))), isEmpty);
    });

    test('one changed character is named by language and key', () {
      final changed = jsonDecode(jsonEncode(same)) as Map<String, dynamic>;
      (changed['th'] as Map<String, dynamic>)['b'] = 'ค';
      expect(differences(changed, same), ['th/b: asset "ค", v2 "ข"']);
    });

    test('a key or a language only one side has is a difference', () {
      final more = jsonDecode(jsonEncode(same)) as Map<String, dynamic>;
      (more['lo'] as Map<String, dynamic>)['c'] = 'x';
      more['my'] = {'a': 'က'};
      expect(differences(more, same), [
        'lo/c: in the asset, not in the v2 fixture',
        'my: in the asset, not in the v2 fixture',
      ]);
      expect(differences(same, more), [
        'lo/c: in the v2 fixture, not in the asset',
        'my: in the v2 fixture, not in the asset',
      ]);
    });

    test('a text against a table is a difference, not a crash', () {
      expect(
        differences(
          {'th': 'x'},
          {
            'th': {'a': 'ก'},
          },
        ),
        hasLength(1),
      );
    });
  });

  group('the kiosk texts against the v2 fixture', () {
    late Map<String, dynamic> asset;
    late Map<String, dynamic> fixture;

    setUpAll(() {
      asset = readJson(KioskL10n.assetPath);
      fixture = readJson(fixturePath);
    });

    test('the fixture says where it came from and holds 16 languages', () {
      expect(fixture['source'], contains('xchange-online@'));
      expect((fixture['text'] as Map<String, dynamic>).length, 16);
    });

    test('every sentence of every language equals the v2 one', () {
      final found = differences(
        asset['text'] as Map<String, dynamic>,
        fixture['text'] as Map<String, dynamic>,
      );
      expect(found, isEmpty, reason: found.join('\n'));
    });

    test('every server error maps to the key v2 mapped it to', () {
      final found = differences(
        asset['errors'] as Map<String, dynamic>,
        fixture['errors'] as Map<String, dynamic>,
      );
      expect(found, isEmpty, reason: found.join('\n'));
    });
  });
}
