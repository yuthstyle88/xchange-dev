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

final dioProvider = Provider<Dio>((ref) {
  final baseUrl = ref.watch(apiBaseUrlProvider);
  final endpoint = ref.watch(kioskEndpointProvider);
  final store = ref.watch(endpointStoreProvider);
  final origin = endpoint != null ? store.originHeader(endpoint) : null;

  final headers = <String, dynamic>{
    'X-Xchange-Request': '1',
    'Origin': ?origin,
  };

  final dio = Dio(
    BaseOptions(
      baseUrl: baseUrl,
      connectTimeout: const Duration(seconds: 10),
      receiveTimeout: const Duration(seconds: 15),
      contentType: Headers.jsonContentType,
      headers: headers,
    ),
  );
  ref.onDispose(dio.close);
  return dio;
});
