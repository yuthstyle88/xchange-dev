import '../entities/kiosk_models.dart';

abstract class KioskBookingRepository {
  Future<BookingCheckInModel> checkInBooking(String qrToken);
}
