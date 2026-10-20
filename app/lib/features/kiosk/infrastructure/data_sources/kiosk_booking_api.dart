import '../../../../core/network/base_api_service.dart';

/// `POST /api/kiosk/bookings/check-in` — booking QR check-in at the kiosk.
class KioskBookingApi extends BaseApiService {
  const KioskBookingApi(super.dio, {required this.branchId});

  static const path = '/api/kiosk/bookings/check-in';
  final String branchId;

  Future<Map<String, dynamic>> checkInBooking(String qrToken) {
    return safeApiCall(() async {
      final response = await dio.post(
        path,
        data: {'branch': branchId, 'qrToken': qrToken},
      );
      final json = response.data;
      if (json is! Map) {
        throw parsingError('checkInBooking answer was not an object');
      }
      return Map<String, dynamic>.from(json);
    }, debugContext: path);
  }
}
