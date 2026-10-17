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
    final failure = failRates;
    final answer = {
      'branchId': 'MAIN',
      'branchName': 'XC-B',
      'open': open,
      'rates': [...rows],
    };
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

  test(
    'row 1: reset read answers when customer is on quote screen and read fails -> error stays empty, screen quote',
    () {
      fakeAsync((async) {
        startAt0(async);
        expect(st().screen, KioskScreen.select);
        expect(st().error, '');

        server.ratesGate = Completer<void>();
        server.failRates = const KioskFailure('');
        c.reset();
        async.flushMicrotasks();

        fillUsd();
        c.submitQuote();
        async.flushMicrotasks();
        expect(st().screen, KioskScreen.quote);
        expect(st().error, '');

        server.ratesGate!.complete();
        async.flushMicrotasks();

        expect(st().screen, KioskScreen.quote);
        expect(st().error, '');
        container.dispose();
      });
    },
  );

  test(
    'row 2: reset read answers when customer is on quote screen and read succeeds with currency gone and counter closed -> currency, amountText, error unchanged, quote kept',
    () {
      fakeAsync((async) {
        startAt0(async);
        expect(st().screen, KioskScreen.select);
        expect(st().error, '');

        server.ratesGate = Completer<void>();
        server.rows = [_jpy];
        server.open = false;
        c.reset();
        async.flushMicrotasks();

        fillUsd();
        c.submitQuote();
        async.flushMicrotasks();
        expect(st().screen, KioskScreen.quote);
        expect(st().currency, 'USD');
        expect(st().amountText, '100');
        expect(st().error, '');
        expect(st().quote, isNotNull);

        server.ratesGate!.complete();
        async.flushMicrotasks();

        expect(st().screen, KioskScreen.quote);
        expect(st().currency, 'USD');
        expect(st().amountText, '100');
        expect(st().error, '');
        expect(st().quote, isNotNull);
        container.dispose();
      });
    },
  );

  test(
    'row 3: after row 2, customer presses Back and 30 s pass -> poller read applies currency=JPY, amountText="", error="closed-en"',
    () {
      fakeAsync((async) {
        startAt0(async);

        server.ratesGate = Completer<void>();
        server.rows = [_jpy];
        server.open = false;
        c.reset();
        async.flushMicrotasks();

        fillUsd();
        c.submitQuote();
        async.flushMicrotasks();

        server.ratesGate!.complete();
        async.flushMicrotasks();

        // On quote screen, form was protected and not modified by late reset read
        expect(st().screen, KioskScreen.quote);
        expect(st().currency, 'USD');
        expect(st().amountText, '100');
        expect(st().error, '');

        c.backToSelect();
        async.flushMicrotasks();
        expect(st().screen, KioskScreen.select);

        server.ratesGate = null;
        async.elapse(const Duration(seconds: 30));
        async.flushMicrotasks();

        expect(st().currency, 'JPY');
        expect(st().amountText, '');
        expect(st().error, 'closed-en');
        expect(st().model!.open, false);
        container.dispose();
      });
    },
  );

  test(
    'row 4: reset read answers when select screen is still up and idle -> applied as today',
    () {
      fakeAsync((async) {
        startAt0(async);

        server.ratesGate = Completer<void>();
        server.rows = [_jpy];
        c.reset();
        async.flushMicrotasks();

        expect(st().screen, KioskScreen.select);
        expect(st().busy, false);
        expect(st().quote, isNull);

        server.ratesGate!.complete();
        async.flushMicrotasks();

        expect(st().currency, 'JPY');
        expect(st().model!.rates.map((r) => r.code).toList(), ['JPY']);
        container.dispose();
      });
    },
  );

  test(
    'row 5: reset read answers when a quote request is in flight (busy on select) -> dropped, next poll applies after quote fails',
    () {
      fakeAsync((async) {
        startAt0(async);

        server.ratesGate = Completer<void>();
        server.quoteGate = Completer<void>();
        server.failQuote = 'quote refused by counter';
        server.rows = [_jpy];
        server.open = false;
        c.reset();
        async.flushMicrotasks();

        fillUsd();
        c.submitQuote();
        async.flushMicrotasks();

        expect(st().screen, KioskScreen.select);
        expect(st().busy, true);

        // Late reset answer arrives while busy submitting quote -> dropped
        server.ratesGate!.complete();
        async.flushMicrotasks();

        expect(st().currency, 'USD');
        expect(st().amountText, '100');
        expect(st().error, '');

        // Quote finishes with refusal error, returns to select screen
        server.quoteGate!.complete();
        async.flushMicrotasks();

        expect(st().screen, KioskScreen.select);
        expect(st().busy, false);
        expect(st().error, 'refused-en');
        expect(st().currency, 'USD');

        // Next poll at 30 s applies new rates
        server.ratesGate = null;
        async.elapse(const Duration(seconds: 30));
        async.flushMicrotasks();

        expect(st().currency, 'JPY');
        expect(st().amountText, '');
        expect(st().error, 'closed-en');
        expect(st().model!.open, false);
        container.dispose();
      });
    },
  );

  test(
    'row 6: reset read answers when screen is waiting or receipt -> dropped',
    () {
      fakeAsync((async) {
        startAt0(async);

        server.ratesGate = Completer<void>();
        server.rows = [_jpy];
        server.open = false;
        c.reset();
        async.flushMicrotasks();

        fillUsd();
        c.submitQuote();
        async.flushMicrotasks();

        // Countdown finishes so canConfirm is true
        async.elapse(const Duration(seconds: 3));
        async.flushMicrotasks();
        expect(st().canConfirm, true);

        c.confirm();
        async.flushMicrotasks();
        expect(st().screen, KioskScreen.waiting);

        // Reset read arrives while on waiting screen -> dropped
        server.ratesGate!.complete();
        async.flushMicrotasks();

        expect(st().screen, KioskScreen.waiting);
        expect(st().error, '');
        container.dispose();
      });
    },
  );

  test('row 7: newer reset() has started since -> dropped by seq guard', () {
    fakeAsync((async) {
      startAt0(async);

      final gate1 = Completer<void>();
      final gate2 = Completer<void>();

      server.ratesGate = gate1;
      c.reset(); // seq 2
      async.flushMicrotasks();

      // Start second reset with different rows
      server.rows = [_jpy];
      server.ratesGate = gate2;
      c.reset(); // seq 3
      async.flushMicrotasks();

      // First reset answers: dropped by seq guard
      gate1.complete();
      async.flushMicrotasks();

      // Second reset answers: applied
      gate2.complete();
      async.flushMicrotasks();

      expect(st().currency, 'JPY');
      container.dispose();
    });
  });
}
