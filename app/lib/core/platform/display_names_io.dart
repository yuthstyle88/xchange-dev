import 'dart:convert';
import 'package:flutter/services.dart' show rootBundle;

import 'display_names.dart';

class BundledDisplayNames implements DisplayNames {
  const BundledDisplayNames.empty()
    : symbols = const {},
      currencies = const {},
      regions = const {};

  const BundledDisplayNames._(this.symbols, this.currencies, this.regions);

  factory BundledDisplayNames.fromJson(Map<String, dynamic> json) {
    final symbols = <String, String>{};
    if (json['symbols'] is Map) {
      for (final entry in (json['symbols'] as Map).entries) {
        symbols['${entry.key}'.toUpperCase()] = '${entry.value}';
      }
    }

    final currencies = <String, Map<String, String>>{};
    if (json['currencies'] is Map) {
      for (final entry in (json['currencies'] as Map).entries) {
        if (entry.value is Map) {
          final map = <String, String>{};
          for (final sub in (entry.value as Map).entries) {
            map['${sub.key}'.toUpperCase()] = '${sub.value}';
          }
          currencies['${entry.key}'] = map;
        }
      }
    }

    final regions = <String, Map<String, String>>{};
    if (json['regions'] is Map) {
      for (final entry in (json['regions'] as Map).entries) {
        if (entry.value is Map) {
          final map = <String, String>{};
          for (final sub in (entry.value as Map).entries) {
            map['${sub.key}'] = '${sub.value}';
          }
          regions['${entry.key}'] = map;
        }
      }
    }

    return BundledDisplayNames._(symbols, currencies, regions);
  }

  static const assetPath = 'assets/l10n/display_names.json';

  static Future<BundledDisplayNames> load() async {
    final raw = await rootBundle.loadString(assetPath);
    return BundledDisplayNames.fromJson(
      jsonDecode(raw) as Map<String, dynamic>,
    );
  }

  final Map<String, String> symbols;
  final Map<String, Map<String, String>> currencies;
  final Map<String, Map<String, String>> regions;

  @override
  String currencySymbol(String code) {
    if (code.isEmpty) return '';
    final upper = code.toUpperCase();
    return symbols[upper] ?? code;
  }

  @override
  String? currencyName(String lang, String code) {
    if (code.isEmpty) return null;
    final upper = code.toUpperCase();
    final byLang = currencies.containsKey(lang)
        ? currencies[lang]
        : currencies['en'];
    if (byLang == null) return null;
    final name = byLang[upper];
    if (name == null || name.toUpperCase() == upper) return null;
    return name;
  }

  @override
  String? regionName(String lang, String region) {
    if (region.isEmpty) return null;
    final byLang = regions.containsKey(lang) ? regions[lang] : regions['en'];
    if (byLang == null) return null;
    final name = byLang[region];
    if (name == null || name == region) return null;
    return name;
  }
}

DisplayNames createDisplayNames() => const BundledDisplayNames.empty();

Future<DisplayNames> loadDisplayNames() => BundledDisplayNames.load();
