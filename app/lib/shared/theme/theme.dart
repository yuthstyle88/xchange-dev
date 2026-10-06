import 'package:flutter/material.dart';

import 'colors.dart';

/// Returns the primary font family for a given language code.
///
/// Simplified and Traditional Chinese, Japanese, and Korean share code points
/// but not glyph shapes. They are kept separate so the theme picks the font
/// by kiosk language rather than using one fallback chain for all.
String? fontForLanguage(String languageCode) {
  return switch (languageCode) {
    'zh' => 'NotoSansSC',
    'zh-HK' => 'NotoSansTC',
    'ja' => 'NotoSansJP',
    'ko' => 'NotoSansKR',
    'th' => 'NotoSansThai',
    'ar' => 'NotoSansArabic',
    'hi' => 'NotoSansDevanagari',
    'lo' => 'NotoSansLao',
    'my' => 'NotoSansMyanmar',
    _ => null,
  };
}

/// Fallback font list based on language.
///
/// Always returns all nine families:
/// - The language's own CJK font first when it has one, then the five non-CJK
///   fonts, then the remaining CJK fonts in the order SC, TC, JP, KR.
/// - For non-CJK languages: the five non-CJK fonts, then the four CJK fonts in
///   the order SC, TC, JP, KR.
List<String> fontFallbacksForLanguage(String languageCode) {
  final ownCjk = switch (languageCode) {
    'zh' => 'NotoSansSC',
    'zh-HK' => 'NotoSansTC',
    'ja' => 'NotoSansJP',
    'ko' => 'NotoSansKR',
    _ => null,
  };

  const nonCjk = [
    'NotoSansThai',
    'NotoSansArabic',
    'NotoSansDevanagari',
    'NotoSansLao',
    'NotoSansMyanmar',
  ];

  const allCjk = ['NotoSansSC', 'NotoSansTC', 'NotoSansJP', 'NotoSansKR'];

  if (ownCjk != null) {
    return [
      ownCjk,
      ...nonCjk,
      for (final f in allCjk)
        if (f != ownCjk) f,
    ];
  }

  return [...nonCjk, ...allCjk];
}

ThemeData kioskTheme(String languageCode) {
  OutlineInputBorder border(Color color) => OutlineInputBorder(
    borderRadius: BorderRadius.circular(14),
    borderSide: BorderSide(color: color, width: 2),
  );

  return ThemeData(
    fontFamily: 'NotoSans',
    fontFamilyFallback: fontFallbacksForLanguage(languageCode),
    colorScheme: ColorScheme.fromSeed(
      seedColor: KioskColors.brand,
      primary: KioskColors.brand,
      surface: Colors.white,
    ),
    scaffoldBackgroundColor: KioskColors.stage,
    inputDecorationTheme: InputDecorationTheme(
      filled: true,
      fillColor: Colors.white,
      border: border(KioskColors.line),
      enabledBorder: border(KioskColors.line),
      focusedBorder: border(KioskColors.brand),
    ),
  );
}
