import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/app/app.dart';
import 'package:xchange_kiosk/features/queue_machine/presentation/pages/queue_machine_home_page.dart';

File _findFile(String relativePath) {
  final candidates = [relativePath, 'app/$relativePath', '../$relativePath'];
  for (final path in candidates) {
    final file = File(path);
    if (file.existsSync()) return file;
  }
  throw StateError('Could not find file: $relativePath');
}

void main() {
  test('pubspec.yaml declares a family Roboto backed by NotoSans', () {
    final pubspecFile = _findFile('pubspec.yaml');
    final content = pubspecFile.readAsStringSync();

    // Verify family Roboto declaration
    final robotoFamilyPattern = RegExp(
      r'-\s+family:\s+Roboto\s*\n\s+fonts:\s*\n\s+-\s+asset:\s+(assets/fonts/NotoSans\.ttf)',
    );
    expect(
      robotoFamilyPattern.hasMatch(content),
      isTrue,
      reason:
          'pubspec.yaml must declare a family named Roboto pointing to assets/fonts/NotoSans.ttf',
    );
  });

  testWidgets(
    'every RichText on queue machine shell has fontFamilyFallback containing NotoSansThai',
    (tester) async {
      tester.view.physicalSize = const Size(1280, 1600);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);

      final container = ProviderContainer();

      container.read(appLanguageProvider.notifier).state = 'th';

      await tester.pumpWidget(
        UncontrolledProviderScope(
          container: container,
          child: const KioskApp(),
        ),
      );
      await tester.pumpAndSettle();

      expect(find.byType(QueueMachineHomePage), findsOneWidget);

      final allRichTexts = tester.widgetList<RichText>(find.byType(RichText));
      expect(allRichTexts, isNotEmpty);
      for (final richText in allRichTexts) {
        if (richText.text.style?.fontFamily == 'MaterialIcons') continue;
        final style = richText.text.style;
        expect(
          style?.fontFamilyFallback,
          contains('NotoSansThai'),
          reason:
              'Every text RichText on shell page must have fontFamilyFallback containing NotoSansThai. Failed text: "${richText.text.toPlainText()}" with style $style',
        );
      }

      container.dispose();
    },
  );
}
