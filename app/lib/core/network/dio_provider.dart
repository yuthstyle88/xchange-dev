import 'package:dio/dio.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../platform/browser.dart' as browser;

/// Where the API is. The XCHANGE server refuses a POST whose `Origin` is not
/// its own host, so the API is always the origin the page was served from.
final apiBaseUrlProvider = Provider<String>((ref) => browser.pageOrigin());

final dioProvider = Provider<Dio>((ref) {
  final dio = Dio(
    BaseOptions(
      baseUrl: ref.watch(apiBaseUrlProvider),
      connectTimeout: const Duration(seconds: 10),
      receiveTimeout: const Duration(seconds: 15),
      contentType: Headers.jsonContentType,
      // The server's cross-site request check (with the Origin comparison).
      headers: const {'X-Xchange-Request': '1'},
    ),
  );
  ref.onDispose(dio.close);
  return dio;
});
