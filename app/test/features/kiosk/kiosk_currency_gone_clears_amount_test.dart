import 'package:fake_async/fake_async.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/features/kiosk/app/kiosk_providers.dart';
import 'package:xchange_kiosk/features/kiosk/domain/entities/kiosk_models.dart';
import 'package:xchange_kiosk/features/kiosk/domain/kiosk_failure.dart';
import 'package:xchange_kiosk/features/kiosk/domain/repository/kiosk_repository.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/controllers/kiosk_controller.dart';
import 'package:xchange_kiosk/l10n/kiosk_l10n.dart';

const _usd = {'code': 'USD', 'nameEn': 'US Dollar', 'buy': 32.1, 'sell': 32.6};
const _jpy = {
  'code': 'JPY',
  'nameEn': 'Japanese Yen',
  'buy': 0.211,
  'sell': 0.219,
};
const _eur = {'code': 'EUR', 'nameEn': 'Euro', 'buy': 35.0, 'sell': 35.5};

class CounterServer implements KioskRepository {
  final calls = <String>[];
  bool open = true;
  List<Map<String, Object>> rows = [_usd, _jpy];
  String serverStatus = 'QUOTED';
  String? receiptImage;

  static const id = '05eb0fcb-d41f-465e-84b6-83a5d26c3ae6';

  Map<String, dynamic> get _identity => {
    'id': id,
    'queue': 'K-05EB0FCB',
    'orderNo': 'ORD-05EB0FCB',
    'transactionNo': null,
    'status': serverStatus,
  };

  @override
  Future<RatesModel> rates() async {
    calls.add('rates');
    return RatesModel.fromJson({
      'branchId': 'MAIN',
      'branchName': 'XC-B',
      'open': open,
      'rates': [...rows],
    });
  }

  @override
  Future<QuoteModel> quote({
    required ExchangeType type,
    required String currency,
    required String amount,
    required String paymentMethod,
  }) async {
    calls.add('quote');
    if (currency.isEmpty || amount.isEmpty) {
      throw const KioskFailure('invalid');
    }
    return QuoteModel.fromJson({
      ..._identity,
      'paymentMethod': paymentMethod,
      'type': type.wire,
      'currency': currency,
      'rate': 32.1,
      'foreign': 100,
      'thb': 3210,
    });
  }

  @override
  Future<RequestIdentity> confirm(String id) async {
    calls.add('confirm');
    serverStatus = 'WAITING';
    return RequestIdentity.fromJson(_identity);
  }

  @override
  Future<RequestIdentity> status(String id) async {
    calls.add('status');
    return RequestIdentity.fromJson(_identity);
  }

  @override
  Future<ReceiptModel> customerReceipt(String id) async {
    calls.add('customerReceipt');
    return ReceiptModel.fromJson({..._identity, 'image': receiptImage});
  }

  @override
  Future<RequestIdentity> customerCancel(String id, String cancelToken) =>
      throw UnimplementedError();
}

KioskL10n _l10n() => KioskL10n.fromJson({
  'langs': [
    {'code': 'en', 'lang': 'en', 'name': 'English'},
    {'code': 'th', 'lang': 'th', 'name': 'ไทย'},
  ],
  'text': {
    'en': {
      'closed': 'closed-en',
      'errGeneric': 'generic-en',
      'quoteExpired': 'expired-en',
      'statusError': 'status-error-en',
    },
    'th': {'closed': 'closed-th', 'errGeneric': 'generic-th'},
  },
  'errors': <String, String>{},
});

void main() {
  late CounterServer server;
  late ProviderContainer container;
  late KioskController c;
  KioskState st() => container.read(kioskControllerProvider);

  KioskController make() {
    container = ProviderContainer(
      overrides: [
        kioskRepositoryProvider.overrideWithValue(server),
        kioskL10nProvider.overrideWithValue(_l10n()),
        deviceLanguagesProvider.overrideWithValue(const ['en']),
      ],
    );
    container.listen(kioskControllerProvider, (_, _) {});
    return container.read(kioskControllerProvider.notifier);
  }

  void startAt0(FakeAsync async) {
    c = make();
    c.start();
    async.flushMicrotasks();
  }

  setUp(() => server = CounterServer());

  test('row 1: JPY with amount 10000, then a refresh without JPY sets '
      'currency USD, amountText empty, estimate null, and Review disabled', () {
    fakeAsync((async) {
      startAt0(async);
      c
        ..setCurrency('JPY')
        ..setAmount('10000')
        ..setPaymentMethod('Cash');
      expect(st().currency, 'JPY');
      expect(st().amountText, '10000');
      expect(st().estimate, isNotNull);
      expect(st().canReview, true);

      server.rows = [_usd];
      async.elapse(const Duration(seconds: 30));
      async.flushMicrotasks();

      expect(st().currency, 'USD');
      expect(st().amountText, '');
      expect(st().estimate, isNull);
      expect(st().canReview, false);
      container.dispose();
    });
  });

  test('row 2: JPY with amount 10000, then a refresh with no rates at all '
      'sets currency null and amountText empty', () {
    fakeAsync((async) {
      startAt0(async);
      c
        ..setCurrency('JPY')
        ..setAmount('10000')
        ..setPaymentMethod('Cash');
      expect(st().currency, 'JPY');
      expect(st().amountText, '10000');

      server.rows = [];
      async.elapse(const Duration(seconds: 30));
      async.flushMicrotasks();

      expect(st().currency, isNull);
      expect(st().amountText, '');
      expect(st().estimate, isNull);
      expect(st().canReview, false);
      container.dispose();
    });
  });

  test('row 3: JPY with 1000 and refresh still offers JPY keeps amount 1000, '
      'whereas a subsequent refresh without JPY clears amountText', () {
    fakeAsync((async) {
      startAt0(async);
      c
        ..setCurrency('JPY')
        ..setAmount('1000')
        ..setPaymentMethod('Cash');

      server.rows = [
        _usd,
        {..._jpy, 'buy': 0.22},
      ];
      async.elapse(const Duration(seconds: 30));
      async.flushMicrotasks();

      expect(st().currency, 'JPY');
      expect(st().amountText, '1000');
      expect(st().estimate, isNotNull);
      expect(st().canReview, true);

      server.rows = [_usd];
      async.elapse(const Duration(seconds: 30));
      async.flushMicrotasks();

      expect(st().currency, 'USD');
      expect(st().amountText, '');
      container.dispose();
    });
  });

  test('row 4: old currency gone moves to the first currency and clears '
      'the typed amount for the removed currency', () {
    fakeAsync((async) {
      startAt0(async);
      c
        ..setCurrency('JPY')
        ..setAmount('10000');
      expect(st().currency, 'JPY');
      expect(st().amountText, '10000');

      server.rows = [_eur, _usd];
      async.elapse(const Duration(seconds: 30));
      async.flushMicrotasks();

      expect(st().currency, 'EUR');
      expect(st().amountText, '');
      container.dispose();
    });
  });

  test('edge case: no amount typed when currency disappears moves to first '
      'currency with empty amountText, and clears amount when next currency '
      'disappears', () {
    fakeAsync((async) {
      startAt0(async);
      c.setCurrency('JPY');
      expect(st().currency, 'JPY');
      expect(st().amountText, '');

      server.rows = [_usd, _eur];
      async.elapse(const Duration(seconds: 30));
      async.flushMicrotasks();

      expect(st().currency, 'USD');
      expect(st().amountText, '');

      c.setAmount('5000');
      expect(st().amountText, '5000');

      server.rows = [_eur];
      async.elapse(const Duration(seconds: 30));
      async.flushMicrotasks();

      expect(st().currency, 'EUR');
      expect(st().amountText, '');
      container.dispose();
    });
  });
}
