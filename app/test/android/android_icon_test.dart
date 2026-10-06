// Provided with ticket #49. These tests are the specification: make them pass,
// do not edit, delete or weaken them.
//
// They read the icon files as they are in the repository. They do not build
// the app; the build is the ticket's own criterion.
import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';

const res = 'android/app/src/main/res';
const foregroundPng = '$res/drawable-nodpi/ic_launcher_foreground.png';
const storeIcon = 'store/play/icon-512.png';

String xml(String path) {
  final file = File(path);
  expect(file.existsSync(), isTrue, reason: '$path is missing');
  return file.readAsStringSync().replaceAll(
    RegExp(r'<!--.*?-->', dotAll: true),
    '',
  );
}

/// An 8-bit RGBA picture, decoded here so the tests need no package.
class Picture {
  Picture(this.width, this.height, this.rgba);
  final int width, height;
  final Uint8List rgba;

  int _at(int x, int y, int c) => rgba[(y * width + x) * 4 + c];
  int alpha(int x, int y) => _at(x, y, 3);

  /// Maroon or grey of the logo: clearly not white.
  bool ink(int x, int y) =>
      alpha(x, y) >= 128 &&
      (_at(x, y, 0) < 200 || _at(x, y, 1) < 200 || _at(x, y, 2) < 200);
  bool white(int x, int y) =>
      alpha(x, y) == 255 &&
      _at(x, y, 0) >= 250 &&
      _at(x, y, 1) >= 250 &&
      _at(x, y, 2) >= 250;

  /// left, top, right, bottom (right and bottom exclusive) of the pixels
  /// [test] accepts, or null when none does.
  List<int>? box(bool Function(int x, int y) test) {
    int? l, t, r, b;
    for (var y = 0; y < height; y++) {
      for (var x = 0; x < width; x++) {
        if (!test(x, y)) continue;
        l = l == null || x < l ? x : l;
        r = r == null || x + 1 > r ? x + 1 : r;
        t ??= y;
        b = y + 1;
      }
    }
    return l == null ? null : [l, t!, r!, b!];
  }
}

Picture png(String path) {
  final file = File(path);
  expect(file.existsSync(), isTrue, reason: '$path is missing');
  final bytes = file.readAsBytesSync();
  final data = ByteData.sublistView(bytes);
  expect(bytes.sublist(0, 8), [
    0x89,
    0x50,
    0x4E,
    0x47,
    0x0D,
    0x0A,
    0x1A,
    0x0A,
  ], reason: '$path is not a PNG file');
  final width = data.getUint32(16), height = data.getUint32(20);
  expect(
    [bytes[24], bytes[25], bytes[28]],
    [8, 6, 0],
    reason:
        '$path must be an 8-bit RGBA PNG (colour type 6), not interlaced, '
        'not a palette or 16-bit picture',
  );
  final packed = BytesBuilder();
  for (var p = 8; p < bytes.length;) {
    final length = data.getUint32(p);
    if (String.fromCharCodes(bytes.sublist(p + 4, p + 8)) == 'IDAT') {
      packed.add(bytes.sublist(p + 8, p + 8 + length));
    }
    p += 12 + length;
  }
  final raw = zlib.decode(packed.takeBytes());
  final stride = width * 4;
  final out = Uint8List(stride * height);
  for (var y = 0; y < height; y++) {
    final filter = raw[y * (stride + 1)];
    for (var i = 0; i < stride; i++) {
      final a = i >= 4 ? out[y * stride + i - 4] : 0;
      final b = y > 0 ? out[(y - 1) * stride + i] : 0;
      final c = i >= 4 && y > 0 ? out[(y - 1) * stride + i - 4] : 0;
      final int predicted;
      switch (filter) {
        case 0:
          predicted = 0;
        case 1:
          predicted = a;
        case 2:
          predicted = b;
        case 3:
          predicted = (a + b) >> 1;
        default:
          final p = a + b - c;
          final pa = (p - a).abs(), pb = (p - b).abs(), pc = (p - c).abs();
          predicted = pa <= pb && pa <= pc ? a : (pb <= pc ? b : c);
      }
      out[y * stride + i] = (raw[y * (stride + 1) + 1 + i] + predicted) & 0xFF;
    }
  }
  return Picture(width, height, out);
}

/// Width divided by height of the logo's ink, measured on the source picture.
double inkShape(Picture p) {
  final b = p.box(p.ink)!;
  return (b[2] - b[0]) / (b[3] - b[1]);
}

void main() {
  group('the adaptive icon (Android 8 and later)', () {
    test('ic_launcher is an adaptive icon made of the two layers', () {
      final x = xml('$res/mipmap-anydpi-v26/ic_launcher.xml');
      expect(x, contains('<adaptive-icon'));
      expect(
        RegExp(
          r'<background\s+android:drawable="([^"]+)"',
        ).firstMatch(x)?.group(1),
        '@color/ic_launcher_background',
      );
      expect(
        RegExp(
          r'<foreground\s+android:drawable="([^"]+)"',
        ).firstMatch(x)?.group(1),
        '@drawable/ic_launcher_foreground',
      );
    });

    test('the background is white (owner, 2026-10-06)', () {
      final x = xml('$res/values/ic_launcher_background.xml');
      final colour = RegExp(
        r'<color\s+name="ic_launcher_background">\s*(#[0-9A-Fa-f]+)\s*</color>',
      ).firstMatch(x)?.group(1);
      expect(colour?.toUpperCase(), anyOf('#FFFFFF', '#FFFFFFFF'));
    });

    test('the foreground is the 108 dp canvas at 4 px per dp', () {
      final p = png(foregroundPng);
      expect([p.width, p.height], [432, 432]);
    });

    test('nothing of the foreground lies outside the 66 dp safe zone, so no '
        'launcher mask can cut the logo', () {
      final p = png(foregroundPng);
      // 66 dp of 108 dp, centred, at 4 px per dp: pixels 84 to 347.
      final b = p.box((x, y) => p.alpha(x, y) > 0);
      expect(b, isNotNull, reason: 'the foreground is empty');
      expect(b![0], greaterThanOrEqualTo(84));
      expect(b[1], greaterThanOrEqualTo(84));
      expect(b[2], lessThanOrEqualTo(348));
      expect(b[3], lessThanOrEqualTo(348));
    });

    test('the foreground shows the logo: as wide as the safe zone, centred, '
        'not stretched', () {
      final p = png(foregroundPng);
      final logo = png('assets/img/logo.png');
      final b = p.box((x, y) => p.alpha(x, y) >= 128)!;
      expect(b[2] - b[0], inInclusiveRange(260, 264));
      expect((b[0] + b[2]) / 2, closeTo(216, 2));
      expect((b[1] + b[3]) / 2, closeTo(216, 2));
      expect(
        (b[2] - b[0]) / (b[3] - b[1]),
        closeTo(logo.width / logo.height, 0.03),
      );
      expect(p.box(p.ink), isNotNull, reason: 'no logo ink in the foreground');
      expect(inkShape(p), closeTo(inkShape(logo), 0.06));
    });
  });

  group('Android 7 and older', () {
    test('the five bitmap icons stay', () {
      for (final d in ['m', 'h', 'xh', 'xxh', 'xxxh']) {
        expect(
          File('$res/mipmap-${d}dpi/ic_launcher.png').existsSync(),
          isTrue,
          reason: 'mipmap-${d}dpi/ic_launcher.png was removed',
        );
      }
    });

    test('the manifest still names @mipmap/ic_launcher and no round icon '
        '(a round icon that exists only for Android 8 would not resolve on '
        'Android 7.1)', () {
      final m = xml('android/app/src/main/AndroidManifest.xml');
      expect(
        RegExp(r'android:icon="([^"]+)"').firstMatch(m)?.group(1),
        '@mipmap/ic_launcher',
      );
      expect(m, isNot(contains('android:roundIcon')));
      expect(
        File('$res/mipmap-anydpi-v26/ic_launcher_round.xml').existsSync(),
        isFalse,
      );
    });
  });

  group('the Play Console listing icon', () {
    test('is 512 x 512, 32-bit PNG, under 1024 KB', () {
      final p = png(storeIcon);
      expect([p.width, p.height], [512, 512]);
      expect(File(storeIcon).lengthSync(), lessThan(1024 * 1024));
    });

    test('is opaque everywhere, white at the four corners', () {
      final p = png(storeIcon);
      expect(
        p.box((x, y) => p.alpha(x, y) != 255),
        isNull,
        reason: 'a transparent pixel would show the store page through it',
      );
      for (final c in [
        [0, 0],
        [511, 0],
        [0, 511],
        [511, 511],
      ]) {
        expect(p.white(c[0], c[1]), isTrue, reason: 'corner $c is not white');
      }
    });

    test('shows the logo three quarters of the width, centred, not '
        'stretched', () {
      final p = png(storeIcon);
      final logo = png('assets/img/logo.png');
      final ink = p.box(p.ink);
      expect(ink, isNotNull, reason: 'the icon is blank');
      final source = logo.box(logo.ink)!;
      // The whole logo is drawn 384 px wide; its ink is this share of it.
      final expected = 384 * (source[2] - source[0]) / logo.width;
      expect(ink![2] - ink[0], closeTo(expected, 8));
      expect((ink[0] + ink[2]) / 2, closeTo(256, 10));
      expect((ink[1] + ink[3]) / 2, closeTo(256, 10));
      expect(inkShape(p), closeTo(inkShape(logo), 0.06));
    });
  });
}
