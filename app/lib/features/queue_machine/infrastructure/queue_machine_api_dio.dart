import 'package:dio/dio.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../core/endpoint/endpoint_store.dart';
import '../../../core/network/dio_provider.dart';
import 'queue_machine_api.dart';
import 'staff_session.dart';

class DioQueueMachineApi implements QueueMachineApi {
  DioQueueMachineApi({
    required this.dio,
    required this.session,
    required this.branchId,
  });

  final Dio dio;
  final StaffSession session;
  final String Function() branchId;

  @override
  Future<void> signIn(String identifier, String password) =>
      session.signIn(identifier, password);

  @override
  Future<IssuedTicket> issueByToken(String token) async {
    try {
      final res = await dio.post<dynamic>(
        '/api/kiosk',
        data: <String, dynamic>{
          'action': 'issueByToken',
          'payload': <String, dynamic>{'branchId': branchId(), 'token': token},
        },
      );
      final data = res.data;
      if (data is Map) {
        final displayNumber = data['displayNumber'];
        final ticketId = data['ticketId'];
        final ticketEscpos = data['ticketEscpos'];
        if (displayNumber is String &&
            ticketId is String &&
            ticketEscpos is String) {
          return IssuedTicket(
            displayNumber: displayNumber,
            ticketId: ticketId,
            ticketEscpos: ticketEscpos,
          );
        }
      }
      throw const QueueMachineRefused('invalidResponse');
    } on DioException catch (err) {
      throw toQueueMachineRefused(err);
    }
  }

  @override
  Future<int> resultSeconds() async {
    try {
      final res = await dio.post<dynamic>(
        '/api/kiosk',
        data: <String, dynamic>{
          'action': 'settings',
          'payload': <String, dynamic>{'branchId': branchId()},
        },
      );
      final data = res.data;
      if (data is Map) {
        final v = data['resultSeconds'] ?? data['result_seconds'];
        if (v is num && v > 0) return v.toInt();
      }
    } catch (_) {
      // Fall back to default 10s when settings are unavailable (§4 D14).
    }
    return 10;
  }
}

QueueMachineApi buildDioQueueMachineApi(Ref ref) {
  final dio = ref.watch(dioProvider);
  final session = ref.watch(staffSessionProvider);
  return DioQueueMachineApi(
    dio: dio,
    session: session,
    branchId: () {
      final endpoint = ref.read(kioskEndpointProvider);
      if (endpoint == null) {
        throw StateError('No kiosk endpoint set');
      }
      return endpoint.branchId;
    },
  );
}
