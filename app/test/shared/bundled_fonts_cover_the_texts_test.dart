import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:xchange_kiosk/shared/theme/theme.dart';
import 'package:flutter_test/flutter_test.dart';

/// Reads all supported Unicode code points from a TTF font's `cmap` table
/// without external packages.
Set<int> readTtfCmap(String filePath) {
  final bytes = File(filePath).readAsBytesSync();
  final data = ByteData.sublistView(bytes);

  final numTables = data.getUint16(4);
  int? cmapOffset;
  for (var i = 0; i < numTables; i++) {
    final recordOffset = 12 + i * 16;
    final tag = String.fromCharCodes(
      bytes.sublist(recordOffset, recordOffset + 4),
    );
    if (tag == 'cmap') {
      cmapOffset = data.getUint32(recordOffset + 8);
      break;
    }
  }

  if (cmapOffset == null) {
    throw StateError('cmap table not found in $filePath');
  }

  final numSubtables = data.getUint16(cmapOffset + 2);
  final supported = <int>{};

  for (var i = 0; i < numSubtables; i++) {
    final subRecordOffset = cmapOffset + 4 + i * 8;
    final subtableOffset = cmapOffset + data.getUint32(subRecordOffset + 4);
    final format = data.getUint16(subtableOffset);

    if (format == 4) {
      // Subtable format 4: Segment mapping to delta values (standard BMP)
      final segCount = data.getUint16(subtableOffset + 6) ~/ 2;
      final endCodesOffset = subtableOffset + 14;
      final startCodesOffset = endCodesOffset + segCount * 2 + 2;
      final idDeltaOffset = startCodesOffset + segCount * 2;
      final idRangeOffsetTableOffset = idDeltaOffset + segCount * 2;

      for (var s = 0; s < segCount; s++) {
        final start = data.getUint16(startCodesOffset + s * 2);
        final end = data.getUint16(endCodesOffset + s * 2);
        final delta = data.getInt16(idDeltaOffset + s * 2);
        final rangeOffset = data.getUint16(idRangeOffsetTableOffset + s * 2);

        for (var c = start; c <= end; c++) {
          if (c == 0xFFFF) break;
          int glyphId;
          if (rangeOffset == 0) {
            glyphId = (c + delta) & 0xFFFF;
          } else {
            final glyphIdPtr =
                idRangeOffsetTableOffset +
                s * 2 +
                rangeOffset +
                (c - start) * 2;
            glyphId = data.getUint16(glyphIdPtr);
            if (glyphId != 0) {
              glyphId = (glyphId + delta) & 0xFFFF;
            }
          }
          if (glyphId > 0) {
            supported.add(c);
          }
        }
      }
    } else if (format == 12) {
      // Subtable format 12: Segmented coverage (Full Unicode 32-bit)
      final numGroups = data.getUint32(subtableOffset + 12);
      for (var g = 0; g < numGroups; g++) {
        final groupOffset = subtableOffset + 16 + g * 12;
        final start = data.getUint32(groupOffset);
        final end = data.getUint32(groupOffset + 4);
        final startGlyphId = data.getUint32(groupOffset + 8);
        for (var c = start; c <= end; c++) {
          if (startGlyphId + (c - start) > 0) {
            supported.add(c);
          }
        }
      }
    }
  }

  return supported;
}

File _findFile(String relativePath) {
  final candidates = [relativePath, 'app/$relativePath', '../$relativePath'];
  for (final path in candidates) {
    final file = File(path);
    if (file.existsSync()) return file;
  }
  throw StateError('Could not find file: $relativePath');
}

Set<String> findMissingCharacters({
  required String fontName,
  required Iterable<String> sentences,
}) {
  final fontFile = _findFile('assets/fonts/$fontName.ttf');
  final supportedCmap = readTtfCmap(fontFile.path);
  final missing = <String>{};
  for (final sentence in sentences) {
    for (final rune in sentence.runes) {
      if (!supportedCmap.contains(rune)) {
        missing.add(String.fromCharCode(rune));
      }
    }
  }
  return missing;
}

void main() {
  test('bundled CJK fonts cover every character of that language\'s texts', () {
    final kioskJsonFile = _findFile('assets/l10n/kiosk.json');
    final kioskData =
        jsonDecode(kioskJsonFile.readAsStringSync()) as Map<String, dynamic>;
    final texts = kioskData['text'] as Map<String, dynamic>;

    const targetLanguages = ['zh', 'zh-HK', 'ja', 'ko'];

    for (final lang in targetLanguages) {
      final fontName = fontForLanguage(lang);
      expect(
        fontName,
        isNotNull,
        reason: 'Theme must define a specific font for language: $lang',
      );

      final theme = kioskTheme(lang);
      expect(
        theme.textTheme.bodyMedium?.fontFamilyFallback?.first,
        equals(fontName),
        reason:
            'Theme for $lang must prioritize $fontName as first fallback to prevent glyph collision across CJK',
      );

      final langTexts = (texts[lang] as Map<String, dynamic>).values
          .map((v) => '$v')
          .toList();

      final missingChars = findMissingCharacters(
        fontName: fontName!,
        sentences: langTexts,
      );

      expect(
        missingChars,
        isEmpty,
        reason:
            'Language "$lang" has characters in kiosk.json missing from $fontName: '
            '${missingChars.join(", ")}. Run `python3 app/tool/subset_fonts.py` to regenerate.',
      );
    }
  });

  test('detects un-subsetted characters and names missing glyphs', () {
    const rareChar = '𠮷';
    final missing = findMissingCharacters(
      fontName: 'NotoSansJP',
      sentences: ['テスト $rareChar'],
    );
    expect(
      missing,
      contains(rareChar),
      reason: 'Should identify glyphs absent from the subset font',
    );
  });

  test('bundled CJK fonts cover every character of browser currency names', () {
    final currencyJsonFile = _findFile('tool/cjk_currency_names.json');
    final currencyData =
        jsonDecode(currencyJsonFile.readAsStringSync()) as Map<String, dynamic>;
    final currencies = currencyData['currencies'] as Map<String, dynamic>;

    const targetLanguages = ['zh', 'zh-HK', 'ja', 'ko'];

    for (final lang in targetLanguages) {
      final fontName = fontForLanguage(lang);
      expect(
        fontName,
        isNotNull,
        reason: 'Theme must define a specific font for language: $lang',
      );

      final langNames = (currencies[lang]['names'] as Map<String, dynamic>)
          .values
          .map((v) => '$v')
          .toList();

      final missingChars = findMissingCharacters(
        fontName: fontName!,
        sentences: langNames,
      );

      expect(
        missingChars,
        isEmpty,
        reason:
            'Language "$lang" has characters in cjk_currency_names.json missing from $fontName: '
            '${missingChars.join(", ")}. Run `python3 app/tool/subset_fonts.py` to regenerate.',
      );
    }
  });
}
