import 'package:dio/dio.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../core/endpoint/endpoint_store.dart';
import '../../../core/endpoint/kiosk_endpoint.dart';
import '../../../core/network/dio_provider.dart';
import '../../../core/platform/device_actions.dart';

/// Whether this app has a setup screen. Off unless start-up turns it on:
/// `main()` sets it to `!kIsWeb`, because the web build takes its endpoint
/// from the page and never shows one (§8.1).
final setupAvailableProvider = Provider<bool>((ref) => false);

/// Whether setup accepts a plain http server: in a debug build only, for
/// testing against a local copy. A release build refuses it (§8.1).
final allowInsecureServerProvider = Provider<bool>((ref) => kDebugMode);

/// The device's languages, most wanted first.
final deviceLanguagesProvider = Provider<List<String>>(
  (ref) => ref.watch(deviceActionsProvider).languages(),
);

/// Probes a server's /health/ready before anything is saved.
typedef SetupProbe = Future<void> Function(KioskEndpoint endpoint);

/// The HTTP client setup tests a server with: the kiosk's own builder, with
/// the `Origin` header the store would send.
final setupDioFactoryProvider = Provider<Dio Function(KioskEndpoint)>((ref) {
  final store = ref.watch(endpointStoreProvider);
  return (endpoint) => buildKioskDio(
    baseUrl: endpoint.origin,
    origin: store.originHeader(endpoint),
  );
});

final setupProbeProvider = Provider<SetupProbe>((ref) {
  final dioFor = ref.watch(setupDioFactoryProvider);
  return (endpoint) async {
    final dio = dioFor(endpoint);
    try {
      final res = await dio.get<dynamic>('/health/ready');
      if (res.statusCode != 200) {
        throw DioException(
          requestOptions: res.requestOptions,
          response: res,
          message:
              'Server refused connection test with status ${res.statusCode}',
        );
      }
    } finally {
      dio.close();
    }
  };
});
