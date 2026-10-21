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
  bool failRates = false;
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
    if (failRates) {
      throw StateError('network down');
    }
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

  test(
    'row 1: reset(), type 10000 while rates load, answer drops JPY -> '
    'currency is first offered, amount empty, estimate null, Review disabled',
    () {
      fakeAsync((async) {
        startAt0(async);
        c.setCurrency('JPY');
        expect(st().currency, 'JPY');

        server.rows = [_usd];
        c.reset();
        c
          ..setAmount('10000')
          ..setPaymentMethod('Cash');
        expect(st().currency, 'JPY');
        expect(st().amountText, '10000');

        async.flushMicrotasks();

        expect(st().currency, 'USD');
        expect(st().amountText, '');
        expect(st().estimate, isNull);
        expect(st().canReview, false);
        container.dispose();
      });
    },
  );

  test(
    'row 2: reset(), type 10000 while rates load, JPY still offered keeps '
    'JPY and 10000, whereas a subsequent reset dropping JPY clears the amount',
    () {
      fakeAsync((async) {
        startAt0(async);
        c.setCurrency('JPY');

        server.rows = [
          _usd,
          {..._jpy, 'buy': 0.22},
        ];
        c.reset();
        c
          ..setAmount('10000')
          ..setPaymentMethod('Cash');

        async.flushMicrotasks();

        expect(st().currency, 'JPY');
        expect(st().amountText, '10000');
        expect(st().estimate, isNotNull);
        expect(st().canReview, true);

        server.rows = [_usd];
        c.reset();
        c
          ..setAmount('10000')
          ..setPaymentMethod('Cash');

        async.flushMicrotasks();

        expect(st().currency, 'USD');
        expect(st().amountText, '');
        expect(st().estimate, isNull);
        expect(st().canReview, false);
        container.dispose();
      });
    },
  );

  test('row 3: refresh dropping the chosen currency clears the amount like '
      'reset() dropping the chosen currency', () {
    fakeAsync((async) {
      startAt0(async);
      c
        ..setCurrency('JPY')
        ..setAmount('10000')
        ..setPaymentMethod('Cash');
      expect(st().canReview, true);

      server.rows = [_usd, _eur];
      async.elapse(const Duration(seconds: 30));
      async.flushMicrotasks();

      expect(st().currency, 'USD');
      expect(st().amountText, '');
      expect(st().estimate, isNull);
      expect(st().canReview, false);

      c.setCurrency('EUR');
      server.rows = [_usd];
      c.reset();
      c
        ..setAmount('5000')
        ..setPaymentMethod('Cash');

      async.flushMicrotasks();

      expect(st().currency, 'USD');
      expect(st().amountText, '');
      expect(st().estimate, isNull);
      expect(st().canReview, false);
      container.dispose();
    });
  });

  test(
    'row 4: reset() with no amount typed keeps empty amount, and reset() '
    'with an amount typed while rates load clears it when the currency is dropped',
    () {
      fakeAsync((async) {
        startAt0(async);
        c
          ..setCurrency('JPY')
          ..setAmount('10000');

        server.rows = [_usd, _jpy];
        c.reset();
        async.flushMicrotasks();

        expect(st().currency, 'JPY');
        expect(st().amountText, '');
        expect(st().estimate, isNull);
        expect(st().canReview, false);

        server.rows = [_eur];
        c.reset();
        c
          ..setAmount('10000')
          ..setPaymentMethod('Cash');
        async.flushMicrotasks();

        expect(st().currency, 'EUR');
        expect(st().amountText, '');
        expect(st().estimate, isNull);
        expect(st().canReview, false);
        container.dispose();
      });
    },
  );

  test('edge case: reset() rates read failure shows the error unchanged, and '
      'when rates recover without the chosen currency during reset() any typed '
      'amount is cleared', () {
    fakeAsync((async) {
      startAt0(async);
      c.setCurrency('JPY');

      server.failRates = true;
      c.reset();
      async.flushMicrotasks();

      expect(st().error, 'generic-en');
      expect(st().currency, 'JPY');
      expect(st().amountText, '');

      server.failRates = false;
      server.rows = [_usd];
      c.reset();
      c
        ..setAmount('10000')
        ..setPaymentMethod('Cash');
      async.flushMicrotasks();

      expect(st().error, '');
      expect(st().currency, 'USD');
      expect(st().amountText, '');
      expect(st().estimate, isNull);
      expect(st().canReview, false);
      container.dispose();
    });
  });
}
