import 'package:dio/dio.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../endpoint/endpoint_store.dart';

/// Where the API is: the endpoint's origin. Throws StateError when there is no endpoint.
final apiBaseUrlProvider = Provider<String>((ref) {
  final endpoint = ref.watch(kioskEndpointProvider);
  if (endpoint == null) {
    throw StateError('No kiosk endpoint set');
  }
  return endpoint.origin;
});

/// The kiosk's HTTP client for one server. [origin] is the `Origin` header,
/// set only where the platform lets a client set it (not in a browser).
/// Shared by [dioProvider] and setup's connection test.
Dio buildKioskDio({required String baseUrl, String? origin}) {
  final headers = <String, dynamic>{
    'X-Xchange-Request': '1',
    'Origin': ?origin,
  };
  return Dio(
    BaseOptions(
      baseUrl: baseUrl,
      connectTimeout: const Duration(seconds: 10),
      receiveTimeout: const Duration(seconds: 15),
      contentType: Headers.jsonContentType,
      headers: headers,
    ),
  );
}

final dioProvider = Provider<Dio>((ref) {
  final baseUrl = ref.watch(apiBaseUrlProvider);
  final endpoint = ref.watch(kioskEndpointProvider);
  final store = ref.watch(endpointStoreProvider);
  final origin = endpoint != null ? store.originHeader(endpoint) : null;

  final dio = buildKioskDio(baseUrl: baseUrl, origin: origin);
  ref.onDispose(dio.close);
  return dio;
});
