/// Wire models for `POST /api/kiosk`. Field names follow the server's JSON.
library;

enum ExchangeType {
  buy('BUY'),
  sell('SELL');

  const ExchangeType(this.wire);
  final String wire;

  static ExchangeType parse(Object? v) =>
      values.firstWhere((e) => e.wire == v, orElse: () => buy);
}

/// Where a kiosk request is in its life. `unknown` keeps a status this client
/// has not heard of from breaking the screen.
enum KioskStatus {
  quoted('QUOTED'),
  waiting('WAITING'),
  accepted('ACCEPTED'),
  idChecked('ID_CHECKED'),
  completed('COMPLETED'),
  expired('EXPIRED'),
  unknown('');

  const KioskStatus(this.wire);
  final String wire;

  static KioskStatus parse(Object? v) =>
      values.firstWhere((e) => e.wire == v, orElse: () => unknown);
}

class Country {
  const Country({required this.code, required this.en, required this.th});
  final String code, en, th;

  factory Country.fromJson(Map<String, dynamic> j) => Country(
    code: '${j['code'] ?? ''}',
    en: '${j['en'] ?? ''}',
    th: '${j['th'] ?? ''}',
  );
}

class CurrencyRate {
  const CurrencyRate({
    required this.code,
    required this.name,
    required this.nameEn,
    required this.buy,
    required this.sell,
    this.countries = const [],
  });
  final String code, name, nameEn;
  final double buy, sell;
  final List<Country> countries;

  double rateFor(ExchangeType type) => type == ExchangeType.buy ? buy : sell;

  factory CurrencyRate.fromJson(Map<String, dynamic> j) => CurrencyRate(
    code: '${j['code']}',
    name: '${j['name'] ?? ''}',
    nameEn: '${j['nameEn'] ?? ''}',
    buy: (j['buy'] as num?)?.toDouble() ?? 0,
    sell: (j['sell'] as num?)?.toDouble() ?? 0,
    countries: [
      for (final c in (j['countries'] as List?) ?? const [])
        Country.fromJson(c as Map<String, dynamic>),
    ],
  );
}

class RatesModel {
  const RatesModel({
    required this.branchId,
    required this.branchName,
    required this.open,
    required this.rates,
  });
  final String branchId, branchName;
  final bool open;
  final List<CurrencyRate> rates;

  factory RatesModel.fromJson(Map<String, dynamic> j) => RatesModel(
    branchId: '${j['branchId'] ?? ''}',
    branchName: '${j['branchName'] ?? ''}',
    open: j['open'] == true,
    rates: [
      for (final r in (j['rates'] as List?) ?? const [])
        CurrencyRate.fromJson(r as Map<String, dynamic>),
    ],
  );
}

/// The identity block every kiosk answer carries.
class RequestIdentity {
  const RequestIdentity({
    required this.id,
    required this.queue,
    required this.orderNo,
    required this.transactionNo,
    required this.status,
    required this.rawStatus,
  });
  final String id, queue, orderNo, rawStatus;
  final String? transactionNo;
  final KioskStatus status;

  factory RequestIdentity.fromJson(Map<String, dynamic> j) => RequestIdentity(
    id: '${j['id']}',
    queue: '${j['queue'] ?? ''}',
    orderNo: '${j['orderNo'] ?? ''}',
    transactionNo: j['transactionNo'] as String?,
    status: KioskStatus.parse(j['status']),
    rawStatus: '${j['status'] ?? ''}',
  );
}

class QuoteModel {
  const QuoteModel({
    required this.identity,
    required this.type,
    required this.currency,
    required this.rate,
    required this.foreign,
    required this.thb,
    required this.paymentMethod,
  });
  final RequestIdentity identity;
  final ExchangeType type;
  final String currency, paymentMethod;
  final double rate, foreign, thb;

  String get id => identity.id;

  factory QuoteModel.fromJson(Map<String, dynamic> j) => QuoteModel(
    identity: RequestIdentity.fromJson(j),
    type: ExchangeType.parse(j['type']),
    currency: '${j['currency']}',
    rate: (j['rate'] as num).toDouble(),
    foreign: (j['foreign'] as num).toDouble(),
    thb: (j['thb'] as num).toDouble(),
    paymentMethod: '${j['paymentMethod'] ?? 'Cash'}',
  );
}

class ReceiptModel {
  const ReceiptModel({required this.identity, required this.image});
  final RequestIdentity identity;

  /// A `data:image/png;base64,…` URL, or null while staff have not published one.
  final String? image;

  factory ReceiptModel.fromJson(Map<String, dynamic> j) => ReceiptModel(
    identity: RequestIdentity.fromJson(j),
    image: j['image'] as String?,
  );
}
