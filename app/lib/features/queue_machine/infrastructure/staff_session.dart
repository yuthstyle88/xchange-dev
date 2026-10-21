import 'package:cookie_jar/cookie_jar.dart';
import 'package:dio/dio.dart';
import 'package:dio_cookie_manager/dio_cookie_manager.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../../core/network/dio_provider.dart';
import 'queue_machine_api.dart';

/// Persists the cookie jar (`__Host-xc_session`, §4.3a W6) in [SharedPreferences].
class SharedPreferencesCookieStorage implements Storage {
  const SharedPreferencesCookieStorage({this.prefix = 'queueMachine.cookie.'});

  final String prefix;

  @override
  Future<void> init(bool persistSession, bool ignoreExpires) async {}

  @override
  Future<String?> read(String key) async {
    final prefs = await SharedPreferences.getInstance();
    return prefs.getString('$prefix$key');
  }

  @override
  Future<void> write(String key, String value) async {
    final prefs = await SharedPreferences.getInstance();
    await prefs.setString('$prefix$key', value);
  }

  @override
  Future<void> delete(String key) async {
    final prefs = await SharedPreferences.getInstance();
    await prefs.remove('$prefix$key');
  }

  @override
  Future<void> deleteAll(List<String> keys) async {
    final prefs = await SharedPreferences.getInstance();
    for (final k in keys) {
      await prefs.remove('$prefix$k');
    }
    for (final k
        in prefs.getKeys().where((k) => k.startsWith(prefix)).toList()) {
      await prefs.remove(k);
    }
  }
}

final staffCookieJarProvider = Provider<CookieJar>(
  (ref) => PersistCookieJar(storage: const SharedPreferencesCookieStorage()),
);

/// Attaches the persisted cookie-jar interceptor on native builds (§4.3a W6).
void attachStaffCookieInterceptor(Dio dio, CookieJar cookieJar) {
  if (!kIsWeb) {
    dio.interceptors.add(CookieManager(cookieJar));
  }
}

/// Maps a [DioException] from auth or kiosk routes to [QueueMachineRefused].
QueueMachineRefused toQueueMachineRefused(DioException err) {
  final status = err.response?.statusCode;
  if (status == 401) {
    return const QueueMachineRefused('unauthorized', statusCode: 401);
  }
  final data = err.response?.data;
  if (data is Map) {
    final error = data['error'];
    if (error is String && error.isNotEmpty) {
      return QueueMachineRefused(error, statusCode: status);
    }
    if (error is Map) {
      final code = error['code'];
      if (code is String && code.isNotEmpty) {
        return QueueMachineRefused(code, statusCode: status);
      }
    }
  }
  if (status == 404) {
    return const QueueMachineRefused('notFound', statusCode: 404);
  }
  return QueueMachineRefused('networkError', statusCode: status);
}

/// Staff session management (§4.3a W6, W12): sign-in, session refresh, and sign-out.
class StaffSession {
  StaffSession(this._dio, {this.cookieJar});

  final Dio _dio;
  final CookieJar? cookieJar;

  Future<void> signIn(String identifier, String password) async {
    try {
      await _dio.post<dynamic>(
        '/api/auth/login',
        data: <String, dynamic>{'identifier': identifier, 'password': password},
      );
    } on DioException catch (err) {
      throw toQueueMachineRefused(err);
    }
  }

  Future<void> refresh() async {
    try {
      await _dio.post<dynamic>(
        '/api/auth/refresh',
        data: const <String, dynamic>{},
      );
    } on DioException catch (err) {
      if (err.response?.statusCode == 401) {
        await cookieJar?.deleteAll();
      }
      throw toQueueMachineRefused(err);
    }
  }

  Future<void> signOut() async {
    try {
      await _dio.post<dynamic>(
        '/api/auth/logout',
        data: const <String, dynamic>{},
      );
    } on DioException catch (err) {
      throw toQueueMachineRefused(err);
    } finally {
      await cookieJar?.deleteAll();
    }
  }
}

final staffSessionProvider = Provider<StaffSession>((ref) {
  final dio = ref.watch(dioProvider);
  final jar = ref.watch(staffCookieJarProvider);
  return StaffSession(dio, cookieJar: jar);
});
