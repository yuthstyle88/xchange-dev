import 'dart:convert';

import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

/// The setup screen's texts: English and Thai, for shop staff.
class SetupL10n {
  const SetupL10n._(this._byLang);

  factory SetupL10n.fromJson(Map<String, dynamic> j) => SetupL10n._({
    for (final e in j.entries)
      e.key: Map<String, String>.from(e.value as Map<String, dynamic>),
  });

  static const asset = 'assets/l10n/setup.json';

  static Future<SetupL10n> load() async => SetupL10n.fromJson(
    jsonDecode(await rootBundle.loadString(asset)) as Map<String, dynamic>,
  );

  final Map<String, Map<String, String>> _byLang;

  /// Thai when the device's first language is Thai, English otherwise.
  static String pick(List<String> deviceLanguages) =>
      deviceLanguages.isNotEmpty &&
          deviceLanguages.first.toLowerCase().startsWith('th')
      ? 'th'
      : 'en';

  String text(String lang, String key) =>
      _byLang[lang]?[key] ?? _byLang['en']?[key] ?? key;
}

final setupL10nProvider = FutureProvider<SetupL10n>((ref) => SetupL10n.load());
