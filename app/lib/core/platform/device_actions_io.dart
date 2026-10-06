import 'dart:convert';
import 'dart:typed_data';
import 'dart:ui' show Locale, PlatformDispatcher;

import 'package:share_plus/share_plus.dart';
import 'package:url_launcher/url_launcher.dart';

import 'device_actions.dart';

typedef LocalesProvider = List<Locale> Function();
typedef UrlLauncher = Future<bool> Function(Uri url);
typedef ImageSharer = Future<void> Function(Uint8List png, String filename);

class NativeDeviceActions extends DeviceActions {
  NativeDeviceActions({
    LocalesProvider? locales,
    UrlLauncher? launch,
    ImageSharer? share,
  }) : _locales = locales ?? _defaultLocales,
       _launch = launch ?? _defaultLaunch,
       _share = share ?? _defaultShare;

  final LocalesProvider _locales;
  final UrlLauncher _launch;
  final ImageSharer _share;

  static List<Locale> _defaultLocales() => PlatformDispatcher.instance.locales;

  static Future<bool> _defaultLaunch(Uri url) =>
      launchUrl(url, mode: LaunchMode.externalApplication);

  static Future<void> _defaultShare(Uint8List png, String filename) async {
    await SharePlus.instance.share(
      ShareParams(
        files: [XFile.fromData(png, mimeType: 'image/png', name: filename)],
      ),
    );
  }

  @override
  List<String> languages() {
    final list = _locales();
    if (list.isEmpty) return const ['en'];
    return list.map((l) => l.toLanguageTag()).toList();
  }

  @override
  Future<void> saveImage(String dataUrl, String filename) async {
    const prefix = 'data:image/png;base64,';
    if (!dataUrl.startsWith(prefix)) {
      throw const DeviceActionException('Expected a PNG data URL');
    }
    final base64String = dataUrl.substring(prefix.length);
    final Uint8List bytes;
    try {
      bytes = base64Decode(base64String);
    } catch (e) {
      throw DeviceActionException('Invalid base64 in PNG data URL', e);
    }
    try {
      await _share(bytes, filename);
    } on DeviceActionException {
      rethrow;
    } catch (e) {
      throw DeviceActionException('Failed to share image', e);
    }
  }

  @override
  Future<void> openHttps(Uri url) async {
    try {
      final ok = await _launch(url);
      if (!ok) {
        throw DeviceActionException('Could not launch $url');
      }
    } on DeviceActionException {
      rethrow;
    } catch (e) {
      throw DeviceActionException('Failed to launch $url', e);
    }
  }
}

DeviceActions createDeviceActions() => NativeDeviceActions();
