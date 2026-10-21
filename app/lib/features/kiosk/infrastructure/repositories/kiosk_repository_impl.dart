import 'package:dio/dio.dart';

import '../../domain/entities/kiosk_models.dart';
import '../../domain/kiosk_failure.dart';
import '../../domain/repository/kiosk_repository.dart';
import '../data_sources/kiosk_api.dart';

class KioskRepositoryImpl implements KioskRepository {
  KioskRepositoryImpl(this._api);

  final KioskApi _api;

  Future<T> _run<T>(
    String action,
    Map<String, Object?> payload,
    T Function(Map<String, dynamic>) parse,
  ) async {
    try {
      return parse(await _api.call(action, payload));
    } on DioException catch (e) {
      final data = e.response?.data;
      throw KioskFailure(data is Map ? '${data['error'] ?? ''}' : '');
    } catch (_) {
      // An answer that does not have the agreed shape.
      throw const KioskFailure('');
    }
  }

  @override
  Future<RatesModel> rates() => _run('rates', const {}, RatesModel.fromJson);

  @override
  Future<QuoteModel> quote({
    required ExchangeType type,
    required String currency,
    required String amount,
    required String paymentMethod,
  }) => _run('quote', {
    'type': type.wire,
    'currency': currency,
    'amount': amount,
    'paymentMethod': paymentMethod,
  }, QuoteModel.fromJson);

  @override
  Future<RequestIdentity> confirm(String id) =>
      _run('confirm', {'id': id}, RequestIdentity.fromJson);

  @override
  Future<RequestIdentity> status(String id) =>
      _run('status', {'id': id}, RequestIdentity.fromJson);

  @override
  Future<ReceiptModel> customerReceipt(String id) =>
      _run('customerReceipt', {'id': id}, ReceiptModel.fromJson);

  @override
  Future<RequestIdentity> customerCancel(String id, String cancelToken) => _run(
    'customerCancel',
    {'id': id, 'cancelToken': cancelToken},
    RequestIdentity.fromJson,
  );
}
