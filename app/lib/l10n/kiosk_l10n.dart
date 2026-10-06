import 'dart:convert';

import 'package:flutter/services.dart' show rootBundle;
import 'package:flutter/widgets.dart' show TextDirection;

class KioskLanguage {
  const KioskLanguage({
    required this.code,
    required this.lang,
    required this.name,
    required this.search,
    required this.rtl,
    this.rank,
  });

  /// Key into the text table (`en`, `zh-HK`, …).
  final String code;

  /// BCP 47 tag used for display names (`zh-Hant`, …).
  final String lang;
  final String name, search;
  final bool rtl;
  final int? rank;

  String get label => rank == null ? name : '$name · #$rank';

  factory KioskLanguage.fromJson(Map<String, dynamic> j) => KioskLanguage(
    code: '${j['code']}',
    lang: '${j['lang'] ?? j['code']}',
    name: '${j['name']}',
    search: '${j['search'] ?? ''}',
    rtl: j['dir'] == 'rtl',
    rank: (j['rank'] as num?)?.toInt(),
  );
}

/// Kiosk texts in every language, moved unchanged from the v2 kiosk's
/// language files.
class KioskL10n {
  KioskL10n({
    required this.languages,
    required this.text,
    required this.errors,
  });

  static const assetPath = 'assets/l10n/kiosk.json';

  final List<KioskLanguage> languages;

  /// Language code → key → sentence.
  final Map<String, Map<String, String>> text;

  /// Server error sentence → text key.
  final Map<String, String> errors;

  factory KioskL10n.fromJson(Map<String, dynamic> j) => KioskL10n(
    languages: [
      for (final l in j['langs'] as List)
        KioskLanguage.fromJson(l as Map<String, dynamic>),
    ],
    text: {
      for (final e in (j['text'] as Map<String, dynamic>).entries)
        e.key: (e.value as Map<String, dynamic>).map(
          (k, v) => MapEntry(k, '$v'),
        ),
    },
    errors: (j['errors'] as Map<String, dynamic>).map(
      (k, v) => MapEntry(k, '$v'),
    ),
  );

  static Future<KioskL10n> load() async => KioskL10n.fromJson(
    jsonDecode(await rootBundle.loadString(assetPath)) as Map<String, dynamic>,
  );

  bool has(String key) => text['en']!.containsKey(key);

  /// The texts in [code]; an unknown code gets English.
  KioskTexts of(String code) =>
      KioskTexts(this, text.containsKey(code) ? code : 'en');

  /// Picks the kiosk language for the device's language list.
  String detect(List<String> deviceLanguages) {
    for (final tag in deviceLanguages) {
      final t = tag.toLowerCase();
      if (RegExp(r'^zh-(hk|mo|tw)|^zh-hant').hasMatch(t)) return 'zh-HK';
      final primary = t.split('-').first;
      if (languages.any((l) => l.code == primary)) return primary;
    }
    return 'en';
  }

  List<KioskLanguage> search(String query) {
    final q = query.trim().toLowerCase();
    return [
      for (final l in languages)
        if ('${l.name} ${l.code} ${l.search}'.toLowerCase().contains(q)) l,
    ];
  }
}

/// [KioskL10n] read in one language.
class KioskTexts {
  const KioskTexts(this._all, this.code);

  final KioskL10n _all;
  final String code;

  KioskLanguage get language => _all.languages.firstWhere(
    (l) => l.code == code,
    orElse: () => _all.languages.firstWhere((l) => l.code == 'en'),
  );
  TextDirection get direction =>
      language.rtl ? TextDirection.rtl : TextDirection.ltr;

  bool has(String key) => _all.has(key);
  List<KioskLanguage> search(String query) => _all.search(query);

  /// Looks [key] up in this language, then English, then returns the key.
  String text(String key, [Map<String, Object?> vars = const {}]) {
    final raw = _all.text[code]?[key] ?? _all.text['en']?[key] ?? key;
    return raw.replaceAllMapped(
      RegExp(r'\{(\w+)\}'),
      (m) => '${vars[m[1]] ?? ''}',
    );
  }

  /// The localised sentence for a server `error`; anything unrecognised gets
  /// the generic sentence rather than the server's own words.
  String errorText(String serverMessage) =>
      text(_all.errors[serverMessage] ?? 'errGeneric');
}
