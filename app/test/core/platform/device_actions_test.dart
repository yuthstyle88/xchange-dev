// Provided with ticket A2. These tests are the specification: make them pass,
// do not edit, delete or weaken them.
//
// On the Dart VM the conditional import picks the device implementation
// (`NativeDeviceActions`). Its three doors to the device — the locale list,
// the link launcher, the share sheet — are constructor arguments, so the rules
// around them are tested here without a device.
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';
import 'dart:ui' show Locale;

import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/core/platform/device_actions.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/pages/waiting_page.dart';
import 'package:xchange_kiosk/l10n/kiosk_l10n.dart';

/// An implementation that only records what reached it.
class _Recording extends DeviceActions {
  final opened = <Uri>[];

  @override
  List<String> languages() => const ['th-TH', 'en'];

  @override
  Future<void> saveImage(String dataUrl, String filename) async {}

  @override
  Future<void> openHttps(Uri url) async => opened.add(url);
}

final _refused = throwsA(isA<DeviceActionException>());

const _notHttps = [
  'http://t.me/XCHANGEXTK',
  'HTTP://t.me/XCHANGEXTK',
  'intent://t.me/XCHANGEXTK#Intent;scheme=https;end',
  'javascript:alert(1)',
  'tel:+6621234567',
  'mailto:someone@example.com',
  'file:///etc/hosts',
  'market://details?id=org.telegram.messenger',
  'data:text/html,hello',
  't.me/XCHANGEXTK',
  '//t.me/XCHANGEXTK',
  '/kiosk',
  'https://',
  'https:///XCHANGEXTK',
  '',
  '   ',
];

const _png =
    'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlE'
    'QVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==';

void main() {
  group('opening a link — the rule every implementation inherits', () {
    test(
      'an https address reaches the implementation once, unchanged',
      () async {
        final d = _Recording();
        await d.openExternal('https://t.me/XCHANGEXTK');
        expect(d.opened.map((u) => '$u'), ['https://t.me/XCHANGEXTK']);
      },
    );

    test('the scheme is judged without regard to case', () async {
      final d = _Recording();
      await d.openExternal('HTTPS://t.me/XCHANGEXTK');
      expect(d.opened.single.scheme, 'https');
      expect(d.opened.single.host, 't.me');
    });

    test('the one link the kiosk has passes', () async {
      final d = _Recording();
      await d.openExternal(WaitingPage.telegramUrl);
      expect(d.opened.map((u) => '$u'), [WaitingPage.telegramUrl]);
    });

    test('anything that is not an absolute https address with a host is '
        'refused, and the implementation never sees it', () async {
      for (final url in _notHttps) {
        final d = _Recording();
        await expectLater(
          () => d.openExternal(url),
          _refused,
          reason: '"$url"',
        );
        expect(d.opened, isEmpty, reason: '"$url"');
      }
    });

    test('a refusal is an Exception with a message', () async {
      Object? caught;
      try {
        await _Recording().openExternal('http://t.me/XCHANGEXTK');
      } catch (e) {
        caught = e;
      }
      expect(caught, isA<DeviceActionException>());
      expect(caught, isA<Exception>());
      expect((caught! as DeviceActionException).message.trim(), isNotEmpty);
    });
  });

  group('device languages', () {
    test('are the device locales as BCP 47 tags, in the device order', () {
      final d = NativeDeviceActions(
        locales: () => const [
          Locale.fromSubtags(
            languageCode: 'zh',
            scriptCode: 'Hant',
            countryCode: 'HK',
          ),
          Locale('th', 'TH'),
          Locale('en'),
        ],
      );
      expect(d.languages(), ['zh-Hant-HK', 'th-TH', 'en']);
    });

    test('the first language the kiosk has wins — the order is not '
        'rearranged', () {
      final l10n = KioskL10n.fromJson(
        jsonDecode(File('assets/l10n/kiosk.json').readAsStringSync())
            as Map<String, dynamic>,
      );
      List<String> of(List<Locale> locales) =>
          NativeDeviceActions(locales: () => locales).languages();

      expect(
        l10n.detect(of(const [Locale('th', 'TH'), Locale('ja', 'JP')])),
        'th',
      );
      expect(
        l10n.detect(of(const [Locale('ja', 'JP'), Locale('th', 'TH')])),
        'ja',
      );
      expect(
        l10n.detect(of(const [Locale('zh', 'TW'), Locale('en', 'US')])),
        'zh-HK',
      );
      // None of the 16: English.
      expect(l10n.detect(of(const [Locale('sv', 'SE')])), 'en');
    });

    test('a device that reports no locale gets English', () {
      expect(NativeDeviceActions(locales: () => const []).languages(), ['en']);
    });

    test('with nothing injected it reads the real device and never answers '
        'an empty list — no binding needed', () {
      final languages = NativeDeviceActions().languages();
      expect(languages, isNotEmpty);
      expect(languages.every((t) => t.trim().isNotEmpty), isTrue);
    });
  });

  group('saving the receipt image', () {
    test('the share sheet gets the decoded PNG and the file name', () async {
      final shared = <(Uint8List, String)>[];
      final d = NativeDeviceActions(
        share: (png, filename) async => shared.add((png, filename)),
      );
      await d.saveImage(_png, 'TX-77_RECEIPT.png');
      expect(shared.length, 1);
      expect(shared.single.$2, 'TX-77_RECEIPT.png');
      expect(
        shared.single.$1,
        base64Decode(_png.substring('data:image/png;base64,'.length)),
      );
      // The PNG signature: the bytes are the image, not the text of the URL.
      expect(shared.single.$1.sublist(0, 4), [0x89, 0x50, 0x4E, 0x47]);
    });

    test('a customer who closes the sheet is not a failure', () async {
      final d = NativeDeviceActions(share: (png, filename) async {});
      await expectLater(d.saveImage(_png, 'a.png'), completes);
    });

    test(
      'something that is not a PNG data URL fails and is never shared',
      () async {
        for (final bad in [
          '',
          'not a data url',
          'https://example.com/receipt.png',
          'data:image/jpeg;base64,AAAA',
          'data:image/png;base64,***not-base64***',
        ]) {
          var calls = 0;
          final d = NativeDeviceActions(
            share: (png, filename) async => calls++,
          );
          await expectLater(
            () => d.saveImage(bad, 'a.png'),
            _refused,
            reason: '"$bad"',
          );
          expect(calls, 0, reason: '"$bad"');
        }
      },
    );

    test('a share sheet that throws is a DeviceActionException carrying the '
        'cause', () async {
      final cause = Exception('no space left on device');
      final d = NativeDeviceActions(
        share: (png, filename) async => throw cause,
      );
      Object? caught;
      try {
        await d.saveImage(_png, 'a.png');
      } catch (e) {
        caught = e;
      }
      expect(caught, isA<DeviceActionException>());
      expect((caught! as DeviceActionException).cause, same(cause));
    });
  });

  group('opening a link on the device', () {
    test('the launcher gets the address and success completes', () async {
      final launched = <Uri>[];
      final d = NativeDeviceActions(
        launch: (url) async {
          launched.add(url);
          return true;
        },
      );
      await d.openExternal('https://t.me/XCHANGEXTK');
      expect(launched.map((u) => '$u'), ['https://t.me/XCHANGEXTK']);
    });

    test('a launcher that answers false is a failure', () async {
      final d = NativeDeviceActions(launch: (url) async => false);
      await expectLater(
        () => d.openExternal('https://t.me/XCHANGEXTK'),
        _refused,
      );
    });

    test('a launcher that throws is a DeviceActionException carrying the '
        'cause', () async {
      final cause = Exception('no activity found');
      final d = NativeDeviceActions(launch: (url) async => throw cause);
      Object? caught;
      try {
        await d.openExternal('https://t.me/XCHANGEXTK');
      } catch (e) {
        caught = e;
      }
      expect(caught, isA<DeviceActionException>());
      expect((caught! as DeviceActionException).cause, same(cause));
    });

    test('a link that is not https never reaches the launcher', () async {
      for (final url in _notHttps) {
        var calls = 0;
        final d = NativeDeviceActions(
          launch: (u) async {
            calls++;
            return true;
          },
        );
        await expectLater(
          () => d.openExternal(url),
          _refused,
          reason: '"$url"',
        );
        expect(calls, 0, reason: '"$url"');
      }
    });
  });

  group('the two implementations, read as source', () {
    // Comments are removed first, so a sentence about a call is not a call.
    String code(String path) {
      final file = File(path);
      expect(file.existsSync(), isTrue, reason: '$path must exist');
      return file
          .readAsLinesSync()
          .where((l) => !l.trimLeft().startsWith('//'))
          .join('\n');
    }

    test('the device one uses url_launcher and share_plus, opens links '
        'outside the app, and does not ask canLaunchUrl first', () {
      final src = code('lib/core/platform/device_actions_io.dart');
      expect(
        src,
        matches(RegExp(r'class NativeDeviceActions extends DeviceActions\b')),
      );
      expect(src, contains("'package:url_launcher/url_launcher.dart'"));
      expect(src, contains("'package:share_plus/share_plus.dart'"));
      expect(src, contains('LaunchMode.externalApplication'));
      expect(src, isNot(contains('canLaunchUrl')));
      expect(src, contains('PlatformDispatcher.instance.locales'));
    });

    test('the browser one keeps what the web build does today: a new tab '
        'without an opener, a download link, the navigator languages — and '
        'no plugin', () {
      final src = code('lib/core/platform/device_actions_web.dart');
      expect(
        src,
        matches(RegExp(r'class BrowserDeviceActions extends DeviceActions\b')),
      );
      expect(src, contains("'_blank'"));
      expect(src, contains("'noopener'"));
      expect(src, contains('download'));
      expect(src, contains('navigator.languages'));
      expect(src, isNot(contains('url_launcher')));
      expect(src, isNot(contains('share_plus')));
    });

    test('both plugins are dependencies of the app', () {
      final pubspec = File('pubspec.yaml').readAsStringSync();
      final dependencies = pubspec.substring(
        pubspec.indexOf('\ndependencies:'),
        pubspec.indexOf('\ndev_dependencies:'),
      );
      expect(
        dependencies,
        matches(RegExp(r'^  url_launcher: \^6\.', multiLine: true)),
      );
      expect(
        dependencies,
        matches(RegExp(r'^  share_plus: \^1[3-9]\.', multiLine: true)),
      );
    });
  });
}
