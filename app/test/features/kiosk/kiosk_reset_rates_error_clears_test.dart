import 'dart:async';

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

class CounterServer implements KioskRepository {
  final calls = <String>[];
  bool open = true;
  List<Map<String, Object>> rows = [_usd, _jpy];
  Object? failRates;
  String? failQuote;
  Completer<void>? ratesGate;
  Completer<void>? quoteGate;
  String serverStatus = 'QUOTED';
  String? receiptImage;

  static const id = '05eb0fcb-d41f-465e-84b6-83a5d26c3ae6';

  int get ratesReads => calls.where((c) => c == 'rates').length;

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
    final gate = ratesGate;
    final answer = {
      'branchId': 'MAIN',
      'branchName': 'XC-B',
      'open': open,
      'rates': [...rows],
    };
    final failure = failRates;
    if (gate != null) await gate.future;
    if (failure != null) throw failure;
    return RatesModel.fromJson(answer);
  }

  @override
  Future<QuoteModel> quote({
    required ExchangeType type,
    required String currency,
    required String amount,
    required String paymentMethod,
  }) async {
    calls.add('quote');
    final gate = quoteGate;
    if (gate != null) await gate.future;
    final refusal = failQuote;
    if (refusal != null) {
      failQuote = null;
      throw KioskFailure(refusal);
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
      'errRefused': 'refused-en',
    },
    'th': {
      'closed': 'closed-th',
      'errGeneric': 'generic-th',
      'errRefused': 'refused-th',
    },
  },
  'errors': <String, String>{'quote refused by counter': 'errRefused'},
});

void main() {
  late CounterServer server;
  late ProviderContainer container;
  late KioskController c;
  KioskState st() => container.read(kioskControllerProvider);
  List<String> codes() => [for (final r in st().model!.rates) r.code];

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

  void fillUsd() {
    c
      ..setCurrency('USD')
      ..setAmount('100')
      ..setPaymentMethod('Cash');
  }

  setUp(() => server = CounterServer());

  test('row 1: rates loaded, then reset() with rates failing, then a refresh '
      'succeeds (open) clears error to empty', () {
    fakeAsync((async) {
      startAt0(async);
      expect(st().model, isNotNull);
      expect(st().error, '');

      server.failRates = const KioskFailure('');
      c.reset();
      async.flushMicrotasks();
      expect(st().model, isNotNull);
      expect(st().error, 'generic-en');

      server.failRates = null;
      async.elapse(const Duration(seconds: 30));
      async.flushMicrotasks();
      expect(st().model!.open, true);
      expect(st().error, '');
      container.dispose();
    });
  });

  test('row 2: rates loaded, then reset() with rates failing, then a refresh '
      'answers closed sets error to closed', () {
    fakeAsync((async) {
      server.open = false;
      startAt0(async);
      expect(st().model!.open, false);
      expect(st().error, 'closed-en');

      server.failRates = const KioskFailure('');
      c.reset();
      async.flushMicrotasks();
      expect(st().model!.open, false);
      expect(st().error, 'generic-en');

      server.failRates = null;
      async.elapse(const Duration(seconds: 30));
      async.flushMicrotasks();
      expect(st().model!.open, false);
      expect(st().error, 'closed-en');
      container.dispose();
    });
  });

  test('row 3: rates loaded, then reset() with rates failing, then a refresh '
      'fails again keeps generic error and last rates until a later refresh '
      'succeeds', () {
    fakeAsync((async) {
      startAt0(async);
      expect(codes(), ['USD', 'JPY']);
      expect(st().error, '');

      server.failRates = const KioskFailure('');
      c.reset();
      async.flushMicrotasks();
      expect(st().error, 'generic-en');
      expect(codes(), ['USD', 'JPY']);

      // Next refresh fails again: generic error stays and last rates are kept.
      async.elapse(const Duration(seconds: 30));
      async.flushMicrotasks();
      expect(server.ratesReads, 3);
      expect(st().error, 'generic-en');
      expect(codes(), ['USD', 'JPY']);

      // Subsequent refresh succeeds: error clears.
      server.failRates = null;
      async.elapse(const Duration(seconds: 30));
      async.flushMicrotasks();
      expect(server.ratesReads, 4);
      expect(st().error, '');
      expect(codes(), ['USD', 'JPY']);
      container.dispose();
    });
  });

  test('row 4: quote refused on select keeps refusal text across refresh with '
      'open unchanged, after recovering from a failed reset rates read', () {
    fakeAsync((async) {
      startAt0(async);

      server.failRates = const KioskFailure('');
      c.reset();
      async.flushMicrotasks();
      expect(st().error, 'generic-en');

      server.failRates = null;
      async.elapse(const Duration(seconds: 30));
      async.flushMicrotasks();
      expect(st().error, '');

      fillUsd();
      server.failQuote = 'quote refused by counter';
      c.activity();
      c.submitQuote();
      async.flushMicrotasks();
      expect(st().screen, KioskScreen.select);
      expect(st().error, 'refused-en');

      async.elapse(const Duration(seconds: 30));
      async.flushMicrotasks();
      expect(st().error, 'refused-en');
      container.dispose();
    });
  });

  test('row 5: a failed refresh adds no error while open, whereas a failed '
      'reset error clears on the next successful refresh', () {
    fakeAsync((async) {
      startAt0(async);
      expect(st().error, '');

      server.failRates = const KioskFailure('');
      async.elapse(const Duration(seconds: 30));
      async.flushMicrotasks();
      expect(st().error, '');
      expect(codes(), ['USD', 'JPY']);

      c.reset();
      async.flushMicrotasks();
      expect(st().error, 'generic-en');

      server.failRates = null;
      async.elapse(const Duration(seconds: 30));
      async.flushMicrotasks();
      expect(st().error, '');
      container.dispose();
    });
  });

  test('row 6: first load fails and next refresh recovers, and a subsequent '
      'failed reset also recovers on the next refresh', () {
    fakeAsync((async) {
      server.failRates = const KioskFailure('');
      startAt0(async);
      expect(st().model, isNull);
      expect(st().error, 'generic-en');

      server.failRates = null;
      async.elapse(const Duration(seconds: 30));
      async.flushMicrotasks();
      expect(st().model, isNotNull);
      expect(st().error, '');

      server.failRates = const KioskFailure('');
      c.reset();
      async.flushMicrotasks();
      expect(st().model, isNotNull);
      expect(st().error, 'generic-en');

      server.failRates = null;
      async.elapse(const Duration(seconds: 30));
      async.flushMicrotasks();
      expect(st().error, '');
      container.dispose();
    });
  });

  test('edge case: a customer action error set after the failed reset read is '
      'not cleared by a later rates success, while a pure failed reset read '
      'error is cleared', () {
    fakeAsync((async) {
      startAt0(async);

      // Failed reset read sets a rates error.
      server.failRates = const KioskFailure('');
      c.reset();
      async.flushMicrotasks();
      expect(st().error, 'generic-en');

      // Customer attempts a quote using the kept model; quote fails with a
      // refusal error, replacing the reset rates error with a customer action
      // error.
      fillUsd();
      server.failQuote = 'quote refused by counter';
      c.activity();
      c.submitQuote();
      async.flushMicrotasks();
      expect(st().error, 'refused-en');

      // Later rates refresh succeeds with open unchanged: customer action error
      // must NOT be cleared.
      server.failRates = null;
      async.elapse(const Duration(seconds: 30));
      async.flushMicrotasks();
      expect(st().error, 'refused-en');

      // When another reset fails and no customer error follows, the next
      // successful refresh clears the reset rates error.
      server.failRates = const KioskFailure('');
      c.reset();
      async.flushMicrotasks();
      expect(st().error, 'generic-en');

      server.failRates = null;
      async.elapse(const Duration(seconds: 30));
      async.flushMicrotasks();
      expect(st().error, '');
      container.dispose();
    });
  });
}
