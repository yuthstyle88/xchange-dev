import '../entities/kiosk_models.dart';

/// The five things a customer's screen can ask the counter's server.
/// Every method throws [KioskFailure] when the server says no.
abstract class KioskRepository {
  Future<RatesModel> rates();

  /// [amount] goes as the customer typed it; the server does the arithmetic.
  Future<QuoteModel> quote({
    required ExchangeType type,
    required String currency,
    required String amount,
    required String paymentMethod,
  });

  Future<RequestIdentity> confirm(String id);
  Future<RequestIdentity> status(String id);
  Future<ReceiptModel> customerReceipt(String id);
}
