import 'package:flutter/foundation.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'device_actions_io.dart'
    if (dart.library.js_interop) 'device_actions_web.dart';

export 'device_actions_io.dart'
    if (dart.library.js_interop) 'device_actions_web.dart';

class DeviceActionException implements Exception {
  const DeviceActionException(this.message, [this.cause]);
  final String message;
  final Object? cause;

  @override
  String toString() => cause == null
      ? 'DeviceActionException: $message'
      : 'DeviceActionException: $message ($cause)';
}

abstract class DeviceActions {
  const DeviceActions();

  List<String> languages();

  Future<void> saveImage(String dataUrl, String filename);

  Future<void> openExternal(String url) async {
    final trimmed = url.trim();
    if (trimmed.isEmpty) {
      throw const DeviceActionException('URL must not be empty');
    }
    final uri = Uri.tryParse(trimmed);
    if (uri == null ||
        !uri.isAbsolute ||
        uri.scheme.toLowerCase() != 'https' ||
        uri.host.isEmpty) {
      throw DeviceActionException('Not an absolute https URL with host: $url');
    }
    await openHttps(uri);
  }

  @protected
  Future<void> openHttps(Uri url);
}

final deviceActionsProvider = Provider<DeviceActions>(
  (ref) => createDeviceActions(),
);
