import 'package:dio/dio.dart';
import 'package:flutter/foundation.dart';

/// All API services extend this class to get:
/// • Automatic safe parsing + stack-trace preserving errors
/// • Centralized logging point (just add logger later)
/// • Consistent error format across the whole app
abstract class BaseApiService {
  final Dio dio;

  const BaseApiService(this.dio);

  /// Use this for every API call inside your services
  Future<T> safeApiCall<T>(
    Future<T> Function() call, {
    String? debugContext,
  }) async {
    try {
      return await call();
    } on DioException {
      // Network / HTTP errors (4xx, 5xx, timeout, etc.) → let them flow naturally
      rethrow;
    } catch (error, stack) {
      // Parsing bugs, unexpected nulls, format changes → convert to DioException
      // so UI layer can handle uniformly via DioException catchers

      if (kDebugMode) {
        debugPrint(
          'API parsing error in $runtimeType${debugContext != null ? ' → $debugContext' : ''}',
        );
        debugPrint('Error: $error');
      }

      final fakeReq = RequestOptions(
        path: debugContext ?? '/${runtimeType.toString().toLowerCase()}',
      );

      Error.throwWithStackTrace(
        DioException(
          requestOptions: fakeReq,
          error: 'parsing-or-unexpected: $error',
          type: DioExceptionType.unknown,
        ),
        stack,
      );
    }
  }

  /// Helper for clean parsing errors
  DioException parsingError(String message, {String? path}) => DioException(
    requestOptions: RequestOptions(
      path: path ?? '/${runtimeType.toString().toLowerCase()}',
    ),
    error: 'json-parsing: $message',
    type: DioExceptionType.unknown,
  );
}
