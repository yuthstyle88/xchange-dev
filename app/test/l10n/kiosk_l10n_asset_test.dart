import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/l10n/kiosk_l10n.dart';

/// Guards the language file moved from the v2 kiosk: every key the screens
/// ask for must exist in English, and every server error must map to a key.
void main() {
  final json =
      jsonDecode(File(KioskL10n.assetPath).readAsStringSync())
          as Map<String, dynamic>;
  final l10n = KioskL10n.fromJson(json);

  test('all 16 languages are present and Arabic is right-to-left', () {
    expect(l10n.languages.length, 16);
    expect(l10n.languages.where((l) => l.rtl).map((l) => l.code), ['ar']);
  });

  test('every text key used in lib/ exists in English', () {
    final used = <String>{};
    for (final f in Directory('lib').listSync(recursive: true)) {
      if (f is! File || !f.path.endsWith('.dart')) continue;
      final src = f.readAsStringSync();
      for (final m in RegExp(r"""\.text\(\s*'(\w+)'""").allMatches(src)) {
        used.add(m[1]!);
      }
      for (final m in RegExp(r"""=> '(phase\w+|step\w+)'""").allMatches(src)) {
        used.add(m[1]!);
      }
    }
    expect(used, isNotEmpty);
    expect(used.where((k) => !l10n.has(k)), isEmpty);
  });

  test('every mapped server error points at an existing key', () {
    final errors = (json['errors'] as Map<String, dynamic>).values;
    expect(errors.where((k) => !l10n.has('$k')), isEmpty);
  });

  test('every status the server can send has a label', () {
    for (final s in [
      'QUOTED',
      'WAITING',
      'ACCEPTED',
      'ID_CHECKED',
      'COMPLETED',
      'EXPIRED',
    ]) {
      expect(l10n.has('st$s'), isTrue, reason: s);
    }
  });
}
