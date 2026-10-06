// Provided with ticket A4. These tests are the specification: make them pass,
// do not edit, delete or weaken them.
//
// They read the Android project as text. They do not build it; the build is
// the ticket's own criterion and, later, CI's (ticket A5).
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

String read(String path) {
  final file = File(path);
  expect(file.existsSync(), isTrue, reason: '$path is missing');
  return file.readAsStringSync();
}

/// The text with XML comments taken out, so a commented-out line proves nothing.
String xml(String path) =>
    read(path).replaceAll(RegExp(r'<!--.*?-->', dotAll: true), '');

/// The text with `//` line comments taken out.
String kts(String path) => read(
  path,
).split('\n').map((l) => l.replaceFirst(RegExp(r'\s*//.*$'), '')).join('\n');

const gradle = 'android/app/build.gradle.kts';
const mainManifest = 'android/app/src/main/AndroidManifest.xml';
const debugManifest = 'android/app/src/debug/AndroidManifest.xml';
const profileManifest = 'android/app/src/profile/AndroidManifest.xml';

void main() {
  group(
    'the store identity (owner, 2026-10-06 — never changed without him)',
    () {
      test('the application id is com.spacing.xchange', () {
        final g = kts(gradle);
        expect(
          RegExp(r'applicationId\s*=\s*"([^"]+)"').firstMatch(g)?.group(1),
          'com.spacing.xchange',
        );
        expect(
          RegExp(r'namespace\s*=\s*"([^"]+)"').firstMatch(g)?.group(1),
          'com.spacing.xchange',
        );
      });

      test('the activity lives in that package', () {
        expect(
          File(
            'android/app/src/main/kotlin/com/spacing/xchange/MainActivity.kt',
          ).existsSync(),
          isTrue,
        );
        expect(
          read(
            'android/app/src/main/kotlin/com/spacing/xchange/MainActivity.kt',
          ),
          contains('package com.spacing.xchange'),
        );
      });

      test('the name under the icon is XC.Xchange', () {
        expect(
          RegExp(
            r'android:label="([^"]+)"',
          ).firstMatch(xml(mainManifest))?.group(1),
          'XC.Xchange',
        );
      });

      test('no generated name is left anywhere in the Android project', () {
        final leftovers = <String>[];
        for (final f in Directory('android').listSync(recursive: true)) {
          if (f is! File) continue;
          if (f.path.contains('/.gradle/') || f.path.contains('/build/')) {
            continue;
          }
          if (!RegExp(r'\.(kts|xml|kt|properties|gradle)$').hasMatch(f.path)) {
            continue;
          }
          final text = f.readAsStringSync();
          if (text.contains('xchange_kiosk') || text.contains('com.example')) {
            leftovers.add(f.path);
          }
        }
        expect(leftovers, isEmpty);
      });
    },
  );

  group('the network', () {
    test('the release app may use the network', () {
      // `flutter create` puts INTERNET only in the debug and profile manifests.
      expect(
        xml(mainManifest),
        contains('<uses-permission android:name="android.permission.INTERNET"'),
      );
    });

    test('the release app does not allow plain http', () {
      expect(xml(mainManifest), isNot(contains('usesCleartextTraffic')));
      expect(xml(mainManifest), isNot(contains('networkSecurityConfig')));
      expect(xml(profileManifest), isNot(contains('usesCleartextTraffic')));
    });

    test('a debug build does, for testing against a local copy', () {
      expect(
        xml(debugManifest),
        contains('android:usesCleartextTraffic="true"'),
      );
    });

    test('the app may ask the system to open an https link', () {
      // Android 11+ package visibility; used by the Telegram button.
      final m = xml(mainManifest);
      final queries = RegExp(
        r'<queries>(.*?)</queries>',
        dotAll: true,
      ).firstMatch(m)?.group(1);
      expect(queries, isNotNull);
      final https = RegExp(r'<intent>(.*?)</intent>', dotAll: true)
          .allMatches(queries!)
          .map((i) => i.group(1)!)
          .where(
            (i) =>
                i.contains('android.intent.action.VIEW') &&
                i.contains('android:scheme="https"'),
          );
      expect(https, isNotEmpty);
    });
  });

  test('the screen turns with the tablet: no orientation is forced', () {
    expect(xml(mainManifest), isNot(contains('android:screenOrientation')));
  });

  group('signing', () {
    test('the release build is signed from android/key.properties when it '
        'exists and falls back to the debug key when it does not', () {
      final g = kts(gradle);
      expect(g, contains('key.properties'));
      expect(g, contains('signingConfigs'));
      expect(g, contains('create("release")'));
      for (final key in [
        'storeFile',
        'storePassword',
        'keyAlias',
        'keyPassword',
      ]) {
        expect(g, contains('getProperty("$key")'), reason: key);
      }
      expect(g, contains('signingConfigs.getByName("debug")'));
    });

    test('no password or keystore path is written in the build file', () {
      final g = kts(gradle);
      expect(
        RegExp(r'(storePassword|keyPassword|keyAlias)\s*=\s*"').hasMatch(g),
        isFalse,
      );
      expect(RegExp(r'\.(jks|keystore)"').hasMatch(g), isFalse);
    });

    test('the secret files can never be committed, and the example says what '
        'to fill in', () {
      final ignore = read('android/.gitignore');
      for (final line in ['key.properties', '**/*.keystore', '**/*.jks']) {
        expect(ignore.split('\n').map((l) => l.trim()), contains(line));
      }
      final example = read('android/key.properties.example');
      for (final key in [
        'storeFile',
        'storePassword',
        'keyAlias',
        'keyPassword',
      ]) {
        expect(example, contains('$key='), reason: key);
      }
      // The example holds no value that could be mistaken for a real one.
      for (final line in example.split('\n')) {
        final m = RegExp(
          r'^(storePassword|keyPassword)=(.*)$',
        ).firstMatch(line);
        if (m != null) expect(m.group(2), anyOf(isEmpty, startsWith('<')));
      }
    });
  });

  test('the sample test that `flutter create` writes was not kept', () {
    expect(File('test/widget_test.dart').existsSync(), isFalse);
  });

  test('app/.metadata lists exactly root, web and android platforms', () {
    final lines = read('.metadata').split('\n');
    final platforms = <String>[];
    var inPlatforms = false;
    for (final line in lines) {
      final trimmed = line.trim();
      if (trimmed == 'platforms:') {
        inPlatforms = true;
        continue;
      }
      if (inPlatforms) {
        if (!line.startsWith(' ') &&
            !line.startsWith('\t') &&
            trimmed.isNotEmpty) {
          inPlatforms = false;
          continue;
        }
        final match = RegExp(r'^-\s+platform:\s*(\S+)').firstMatch(trimmed);
        if (match != null) {
          platforms.add(match.group(1)!);
        } else if (trimmed.endsWith(':') && !trimmed.startsWith('-')) {
          inPlatforms = false;
        }
      }
    }
    expect(platforms, ['root', 'web', 'android']);
  });
}
