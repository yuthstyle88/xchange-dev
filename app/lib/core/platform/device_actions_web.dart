import 'dart:js_interop';

import 'package:web/web.dart' as web;

import 'device_actions.dart';

class BrowserDeviceActions extends DeviceActions {
  const BrowserDeviceActions();

  @override
  List<String> languages() {
    final list = web.window.navigator.languages.toDart
        .map((l) => l.toDart)
        .toList();
    return list.isEmpty ? [web.window.navigator.language] : list;
  }

  @override
  Future<void> saveImage(String dataUrl, String filename) async {
    final a = web.document.createElement('a') as web.HTMLAnchorElement
      ..href = dataUrl
      ..download = filename;
    web.document.body!.append(a);
    a.click();
    a.remove();
  }

  @override
  Future<void> openHttps(Uri url) async {
    web.window.open(url.toString(), '_blank', 'noopener');
  }
}

DeviceActions createDeviceActions() => const BrowserDeviceActions();
