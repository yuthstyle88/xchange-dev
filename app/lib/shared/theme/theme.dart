import 'package:flutter/material.dart';

import 'colors.dart';

ThemeData kioskTheme() {
  OutlineInputBorder border(Color color) => OutlineInputBorder(
    borderRadius: BorderRadius.circular(14),
    borderSide: BorderSide(color: color, width: 2),
  );
  return ThemeData(
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
