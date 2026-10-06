// Provided with ticket A2. These tests are the specification: make them pass,
// do not edit, delete or weaken them.
//
// After A2 nothing is left of `core/platform/browser*.dart`, and each target
// is built only from code that exists on it. The tests read `lib/` as text;
// each one also proves that it looked at something.
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

final _directive = RegExp(
  r"^\s*(?:import|export)\s+'([^']+)'((?:\s+if\s*\([^)]*\)\s*'[^']+')*)",
  multiLine: true,
);
final _branch = RegExp(r"if\s*\(\s*([\w.]+)[^)]*\)\s*'([^']+)'");

/// The URIs [source] imports or exports when built for the web
/// (`dart.library.js_interop` / `dart.library.html` are true) or for a device
/// (`dart.library.io` is true).
List<String> directives(String source, {required bool web}) {
  final truths = web
      ? const {'dart.library.js_interop', 'dart.library.html'}
      : const {'dart.library.io'};
  final uris = <String>[];
  for (final m in _directive.allMatches(source)) {
    var uri = m[1]!;
    for (final b in _branch.allMatches(m[2] ?? '')) {
      if (truths.contains(b[1])) {
        uri = b[2]!;
        break;
      }
    }
    uris.add(uri);
  }
  return uris;
}

class Build {
  /// Files of this app reached from `lib/main.dart`, as paths under `lib/`.
  final files = <String>{};

  /// Everything else those files import: `dart:…` and other packages.
  final external = <String>{};
}

Build buildFor({required bool web}) {
  final build = Build();
  final queue = ['lib/main.dart'];
  while (queue.isNotEmpty) {
    final path = queue.removeLast();
    if (!build.files.add(path)) {
      continue;
    }
    final file = File(path);
    expect(file.existsSync(), isTrue, reason: '$path is imported but missing');
    for (final uri in directives(file.readAsStringSync(), web: web)) {
      if (uri.startsWith('package:xchange_kiosk/')) {
        queue.add('lib/${uri.substring('package:xchange_kiosk/'.length)}');
      } else if (uri.contains(':')) {
        build.external.add(uri);
      } else {
        queue.add(File(path).parent.uri.resolve(uri).toFilePath());
      }
    }
  }
  return build;
}

List<File> libFiles() => [
  for (final f in Directory('lib').listSync(recursive: true))
    if (f is File && f.path.endsWith('.dart')) f,
];

const _browserOnly = [
  'dart:js_interop',
  'dart:html',
  'dart:js',
  'dart:js_util',
];

bool _isBrowserOnly(String uri) =>
    _browserOnly.contains(uri) ||
    uri.startsWith('dart:js_interop') ||
    uri.startsWith('package:web/');

void main() {
  test('the reader of imports understands conditional imports', () {
    const source = '''
import 'dart:convert';
import 'package:flutter/material.dart';

import 'a_io.dart'
    if (dart.library.js_interop) 'a_web.dart';
export 'b_stub.dart' if (dart.library.io) 'b_io.dart';
// import 'commented_out.dart';
import '../x/y.dart' as y;
''';
    expect(directives(source, web: true), [
      'dart:convert',
      'package:flutter/material.dart',
      'a_web.dart',
      'b_stub.dart',
      '../x/y.dart',
    ]);
    expect(directives(source, web: false), [
      'dart:convert',
      'package:flutter/material.dart',
      'a_io.dart',
      'b_io.dart',
      '../x/y.dart',
    ]);
  });

  group('the browser seam', () {
    test('its three files are deleted', () {
      // The directory itself is still there, with the new files in it.
      expect(Directory('lib/core/platform').existsSync(), isTrue);
      for (final name in [
        'browser.dart',
        'browser_stub.dart',
        'browser_web.dart',
      ]) {
        expect(
          File('lib/core/platform/$name').existsSync(),
          isFalse,
          reason: name,
        );
      }
    });

    test('core/platform holds exactly the six files of the two seams', () {
      final names = [
        for (final f in Directory('lib/core/platform').listSync())
          f.uri.pathSegments.lastWhere((s) => s.isNotEmpty),
      ]..sort();
      expect(names, [
        'device_actions.dart',
        'device_actions_io.dart',
        'device_actions_web.dart',
        'display_names.dart',
        'display_names_io.dart',
        'display_names_web.dart',
      ]);
    });

    test('nothing in lib/ imports it or uses one of its functions', () {
      final gone = RegExp(
        r'platform/browser|browser_stub|browser_web|'
        r'\b(saveDataUrl|currencyDisplayName|regionDisplayName|pageOrigin|'
        r'branchFromUrl)\b|'
        r'\b(browser|platform)\.(deviceLanguages|currencySymbol|openExternal)\b',
      );
      // The pattern does find the seam as it was written before this ticket.
      for (final old in [
        "import '../../../core/platform/browser.dart' as browser;",
        "export 'browser_stub.dart' if (dart.library.js_interop) 'browser_web.dart';",
        '(ref) => browser.deviceLanguages(),',
        'onPressed: () => platform.saveDataUrl(image, filename(id)),',
        'onPressed: () => platform.openExternal(telegramUrl),',
        "platform.regionDisplayName(lang, k.code) ?? '',",
        r"'${platform.currencySymbol(code)} $code'",
        'platform.currencyDisplayName(l10n.language.lang, r.code) ??',
      ]) {
        expect(gone.hasMatch(old), isTrue, reason: old);
      }

      final files = libFiles();
      expect(files.length, greaterThan(25));
      final hits = [
        for (final f in files)
          for (final m in gone.allMatches(f.readAsStringSync()))
            '${f.path}: ${m[0]}',
      ];
      expect(hits, isEmpty);
    });

    test('browser-only libraries are imported only by *_web.dart files, and '
        'dart:io only by *_io.dart files', () {
      final files = libFiles();
      expect(files.length, greaterThan(25));
      var webFiles = 0;
      for (final f in files) {
        final uris = {
          ...directives(f.readAsStringSync(), web: true),
          ...directives(f.readAsStringSync(), web: false),
        };
        if (f.path.endsWith('_web.dart')) {
          webFiles++;
        } else {
          expect(uris.where(_isBrowserOnly), isEmpty, reason: f.path);
        }
        if (!f.path.endsWith('_io.dart')) {
          expect(uris, isNot(contains('dart:io')), reason: f.path);
        }
      }
      // endpoint_store_web, display_names_web, device_actions_web.
      expect(webFiles, 3);
    });
  });

  group('what each target is built from', () {
    test('the device build: the three *_io.dart files, no *_web.dart file, '
        'no browser library', () {
      final build = buildFor(web: false);
      expect(build.files.length, greaterThan(25));
      expect(
        build.files,
        containsAll([
          'lib/main.dart',
          'lib/features/kiosk/presentation/pages/select_page.dart',
          'lib/features/kiosk/presentation/pages/receipt_page.dart',
          'lib/features/kiosk/presentation/pages/waiting_page.dart',
          'lib/core/platform/display_names.dart',
          'lib/core/platform/display_names_io.dart',
          'lib/core/platform/device_actions.dart',
          'lib/core/platform/device_actions_io.dart',
          'lib/core/endpoint/endpoint_store_io.dart',
        ]),
      );
      expect(build.files.where((f) => f.endsWith('_web.dart')), isEmpty);
      expect(build.external.where(_isBrowserOnly), isEmpty);
      expect(
        build.external,
        containsAll([
          'package:url_launcher/url_launcher.dart',
          'package:share_plus/share_plus.dart',
        ]),
      );
    });

    test('the web build: the three *_web.dart files, no *_io.dart file, no '
        'dart:io, and neither plugin — it keeps using the browser itself', () {
      final build = buildFor(web: true);
      expect(build.files.length, greaterThan(25));
      expect(
        build.files,
        containsAll([
          'lib/main.dart',
          'lib/features/kiosk/presentation/pages/select_page.dart',
          'lib/features/kiosk/presentation/pages/receipt_page.dart',
          'lib/features/kiosk/presentation/pages/waiting_page.dart',
          'lib/core/platform/display_names.dart',
          'lib/core/platform/display_names_web.dart',
          'lib/core/platform/device_actions.dart',
          'lib/core/platform/device_actions_web.dart',
          'lib/core/endpoint/endpoint_store_web.dart',
        ]),
      );
      expect(build.files.where((f) => f.endsWith('_io.dart')), isEmpty);
      expect(build.external, isNot(contains('dart:io')));
      expect(build.external, isNot(contains('dart:ffi')));
      expect(
        build.external.where(
          (u) =>
              u.startsWith('package:url_launcher') ||
              u.startsWith('package:share_plus'),
        ),
        isEmpty,
      );
      expect(build.external.where(_isBrowserOnly), isNotEmpty);
    });

    test('the two builds differ in exactly those six files', () {
      final device = buildFor(web: false).files;
      final web = buildFor(web: true).files;
      expect(device.difference(web), {
        'lib/core/endpoint/endpoint_store_io.dart',
        'lib/core/platform/display_names_io.dart',
        'lib/core/platform/device_actions_io.dart',
      });
      expect(web.difference(device), {
        'lib/core/endpoint/endpoint_store_web.dart',
        'lib/core/platform/display_names_web.dart',
        'lib/core/platform/device_actions_web.dart',
      });
    });
  });
}
