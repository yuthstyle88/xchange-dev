import 'package:dio/dio.dart';

import '../../domain/entities/kiosk_models.dart';
import '../../domain/kiosk_failure.dart';
import '../../domain/repository/kiosk_booking_repository.dart';
import '../data_sources/kiosk_booking_api.dart';

class KioskBookingRepositoryImpl implements KioskBookingRepository {
  KioskBookingRepositoryImpl(this._api);

  final KioskBookingApi _api;

  @override
  Future<BookingCheckInModel> checkInBooking(String qrToken) async {
    try {
      final json = await _api.checkInBooking(qrToken);
      return BookingCheckInModel.fromJson(json);
    } on DioException catch (e) {
      final data = e.response?.data;
      if (data is Map) {
        final err = data['error'];
        if (err is Map) {
          throw KioskFailure('${err['code'] ?? ''}');
        }
        if (err is String) {
          throw KioskFailure(err);
        }
      }
      throw const KioskFailure('');
    } catch (_) {
      throw const KioskFailure('');
    }
  }
}
