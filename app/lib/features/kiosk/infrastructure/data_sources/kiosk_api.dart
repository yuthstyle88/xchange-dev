import '../../../../core/network/base_api_service.dart';

/// `POST /api/kiosk` — one endpoint, the action named in the body.
class KioskApi extends BaseApiService {
  const KioskApi(super.dio, {required this.branchId});

  static const path = '/api/kiosk';
  final String branchId;

  Future<Map<String, dynamic>> call(
    String action, [
    Map<String, Object?> payload = const {},
  ]) {
    return safeApiCall(() async {
      final response = await dio.post(
        path,
        data: {
          'action': action,
          'payload': {...payload, 'branchId': branchId},
        },
      );
      final json = response.data;
      if (json is! Map) throw parsingError('$action answer was not an object');
      return Map<String, dynamic>.from(json);
    }, debugContext: '$path $action');
  }
}
