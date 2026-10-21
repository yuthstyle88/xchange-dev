// Provided with the ticket "the kiosk keeps showing closed after the counter
// opens", written before the fix. Do not edit, delete or weaken these tests;
// add your own in another file.
//
// While the select screen is up, the controller re-reads `rates` every 30 s so
// the open/closed state and the rates follow the counter without a restart.
// v2's `public/kiosk.js` reads them once per reset; this is a decided
// difference (docs/design/xchange-v3-known-differences.md).
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

/// The counter's server as the kiosk sees it, with the knobs these tests turn:
/// whether the counter is open, which rates it offers, a failure for the next
/// rates read, and gates that hold a rates read or a quote in flight.
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
    // Read the answer when the call is made, as the server would.
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
  List<String> codes() => [for (final r in st().model!.rates) r.code];

  KioskController make() {
    container = ProviderContainer(
      overrides: [
        kioskRepositoryProvider.overrideWithValue(server),
        kioskL10nProvider.overrideWithValue(_l10n()),
        deviceLanguagesProvider.overrideWithValue(const ['en']),
      ],
    );
    // Keep the controller alive the way the page's watch does.
    container.listen(kioskControllerProvider, (_, _) {});
    return container.read(kioskControllerProvider.notifier);
  }

  /// Starts the kiosk at t = 0 and lets the first rates read land.
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

  test('a counter that opens while the select screen is up shows open within '
      '30 s, with no restart and no touch', () {
    fakeAsync((async) {
      server.open = false;
      startAt0(async);
      fillUsd();
      expect(st().error, 'closed-en');
      expect(st().model!.open, false);
      expect(st().canReview, false);

      server.open = true;
      async.elapse(const Duration(seconds: 29));
      expect(server.ratesReads, 1, reason: 'no second read before 30 s');
      expect(st().error, 'closed-en');

      async.elapse(const Duration(seconds: 1));
      async.flushMicrotasks();
      expect(server.ratesReads, 2);
      expect(st().model!.open, true);
      expect(st().error, '');
      expect(st().canReview, true);
      expect(st().screen, KioskScreen.select);
      container.dispose();
    });
  });

  test('a counter that closes while the select screen is up shows closed '
      'within 30 s', () {
    fakeAsync((async) {
      startAt0(async);
      fillUsd();
      expect(st().error, '');
      expect(st().canReview, true);

      server.open = false;
      async.elapse(const Duration(seconds: 30));
      async.flushMicrotasks();
      expect(st().model!.open, false);
      expect(st().error, 'closed-en');
      expect(st().canReview, false);
      container.dispose();
    });
  });

  test('the refresh brings the new rates, keeps the chosen currency while it '
      'is offered, and moves to the first one when it is gone', () {
    fakeAsync((async) {
      startAt0(async);
      c.setCurrency('JPY');

      server.rows = [
        _usd,
        {..._jpy, 'buy': 0.25},
      ];
      async.elapse(const Duration(seconds: 30));
      async.flushMicrotasks();
      expect(st().currency, 'JPY');
      expect(st().selectedRate!.buy, 0.25);

      server.rows = [_usd];
      async.elapse(const Duration(seconds: 30));
      async.flushMicrotasks();
      expect(codes(), ['USD']);
      expect(st().currency, 'USD', reason: 'the same rule as reset()');
      container.dispose();
    });
  });

  test('the refresh leaves the language, type, amount and payment method the '
      'customer chose, and speaks the chosen language', () {
    fakeAsync((async) {
      startAt0(async);
      c
        ..setLanguage('th')
        ..setType(ExchangeType.sell)
        ..setCurrency('JPY')
        ..setAmount('1000')
        ..setPaymentMethod('Alipay');

      server.open = false;
      async.elapse(const Duration(seconds: 30));
      async.flushMicrotasks();
      expect(st().error, 'closed-th');
      expect(st().languageCode, 'th');
      expect(st().type, ExchangeType.sell);
      expect(st().currency, 'JPY');
      expect(st().amountText, '1000');
      expect(st().paymentMethod, 'Alipay');
      expect(st().screen, KioskScreen.select);
      container.dispose();
    });
  });

  test('the refresh is not activity: two idle minutes still reset the '
      'screen', () {
    fakeAsync((async) {
      startAt0(async);
      c.setLanguage('th');

      server.open = false;
      async.elapse(const Duration(seconds: 30));
      async.flushMicrotasks();
      expect(st().error, 'closed-th');

      // The last touch was the start (t = 0); refreshes at 30, 60 and 90 s
      // must not postpone the idle reset due at 120 s.
      async.elapse(const Duration(seconds: 89));
      expect(st().languageCode, 'th', reason: 't = 119 s');
      async.elapse(const Duration(seconds: 2));
      async.flushMicrotasks();
      expect(st().languageCode, 'en', reason: 't = 121 s: idle reset ran');
      container.dispose();
    });
  });

  test('no refresh on the quote screen; it resumes within 30 s of going back '
      'to select', () {
    fakeAsync((async) {
      startAt0(async);
      async.elapse(const Duration(seconds: 30));
      async.flushMicrotasks();
      expect(server.ratesReads, 2, reason: 'refreshed once on select');

      fillUsd();
      c.activity();
      c.submitQuote();
      async.flushMicrotasks();
      expect(st().screen, KioskScreen.quote);
      async.elapse(const Duration(seconds: 61));
      expect(server.ratesReads, 2, reason: 'none on the quote screen');

      c.backToSelect();
      c.activity();
      async.elapse(const Duration(seconds: 30));
      async.flushMicrotasks();
      expect(server.ratesReads, 3);
      container.dispose();
    });
  });

  // #4243 ("ระบบ kiosk มัน จะเริ่มใหม่ เมื่อ ทำจบ" · "ให้ตั้งค่าได้เอง"):
  // hold the result window past the 60 s checks so we stay on waiting/receipt.
  test('no refresh on the waiting and receipt screens', () {
    fakeAsync((async) {
      container = ProviderContainer(
        overrides: [
          kioskRepositoryProvider.overrideWithValue(server),
          kioskL10nProvider.overrideWithValue(_l10n()),
          deviceLanguagesProvider.overrideWithValue(const ['en']),
          kioskResultSecondsProvider.overrideWithValue(600),
        ],
      );
      container.listen(kioskControllerProvider, (_, _) {});
      c = container.read(kioskControllerProvider.notifier);
      c.start();
      async.flushMicrotasks();
      async.elapse(const Duration(seconds: 30));
      async.flushMicrotasks();
      expect(server.ratesReads, 2, reason: 'refreshed once on select');

      fillUsd();
      c.activity();
      c.submitQuote();
      async.flushMicrotasks();
      async.elapse(const Duration(seconds: 3));
      c.confirm();
      async.flushMicrotasks();
      expect(st().screen, KioskScreen.waiting);
      async.elapse(const Duration(seconds: 60));
      expect(server.ratesReads, 2, reason: 'none on the waiting screen');

      server
        ..serverStatus = 'ID_CHECKED'
        ..receiptImage = 'data:image/png;base64,AAAA';
      async.elapse(const Duration(seconds: 3));
      expect(st().screen, KioskScreen.receipt);
      c.activity();
      async.elapse(const Duration(seconds: 60));
      expect(server.ratesReads, 2, reason: 'none on the receipt screen');
      container.dispose();
    });
  });

  test('no refresh while a quote is being asked for', () {
    fakeAsync((async) {
      startAt0(async);
      async.elapse(const Duration(seconds: 30));
      async.flushMicrotasks();
      expect(server.ratesReads, 2, reason: 'refreshed once on select');

      fillUsd();
      c.activity();
      server.quoteGate = Completer<void>();
      c.submitQuote();
      async.flushMicrotasks();
      expect(st().busy, true);
      expect(st().screen, KioskScreen.select);
      async.elapse(const Duration(seconds: 60));
      expect(server.ratesReads, 2, reason: 'none while busy');

      server.quoteGate!.complete();
      async.flushMicrotasks();
      expect(st().screen, KioskScreen.quote);
      expect(st().busy, false);
      container.dispose();
    });
  });

  test('a failed refresh keeps the last rates and adds no error; a later one '
      'recovers silently', () {
    fakeAsync((async) {
      startAt0(async);
      c.setCurrency('JPY');

      // The repository turns a network failure into KioskFailure('') and a
      // server refusal into KioskFailure(<its sentence>).
      for (final failure in <Object>[
        const KioskFailure(''),
        const KioskFailure('Internal Server Error'),
        StateError('anything else'),
      ]) {
        server.failRates = failure;
        c.activity();
        async.elapse(const Duration(seconds: 30));
        async.flushMicrotasks();
        expect(st().error, '', reason: '$failure while open');
        expect(st().model!.open, true);
        expect(codes(), ['USD', 'JPY']);
        expect(st().currency, 'JPY');
      }

      server
        ..failRates = null
        ..open = false;
      c.activity();
      async.elapse(const Duration(seconds: 30));
      async.flushMicrotasks();
      expect(st().error, 'closed-en');

      server.failRates = const KioskFailure('');
      c.activity();
      async.elapse(const Duration(seconds: 30));
      async.flushMicrotasks();
      expect(st().error, 'closed-en', reason: 'a failure while closed');
      expect(st().model!.open, false);

      server
        ..failRates = null
        ..open = true;
      c.activity();
      async.elapse(const Duration(seconds: 30));
      async.flushMicrotasks();
      expect(st().error, '');
      expect(st().model!.open, true);
      container.dispose();
    });
  });

  test('a kiosk whose first rates read failed recovers on the next refresh; '
      'an error the customer is reading stays while the counter stays '
      'open', () {
    fakeAsync((async) {
      server.failRates = const KioskFailure('');
      startAt0(async);
      expect(st().model, isNull);
      expect(st().error, 'generic-en');

      server.failRates = null;
      async.elapse(const Duration(seconds: 30));
      async.flushMicrotasks();
      expect(st().model, isNotNull);
      expect(codes(), ['USD', 'JPY']);
      expect(st().currency, 'USD');
      expect(st().error, '');

      fillUsd();
      server.failQuote = 'a refusal the client has never heard of';
      c.activity();
      c.submitQuote();
      async.flushMicrotasks();
      expect(st().screen, KioskScreen.select);
      expect(st().error, 'generic-en');

      async.elapse(const Duration(seconds: 30));
      async.flushMicrotasks();
      expect(server.ratesReads, 3);
      expect(
        st().error,
        'generic-en',
        reason: 'open did not change, so the refresh leaves the error',
      );
      container.dispose();
    });
  });

  test('a refresh still in flight is not doubled, and its answer is dropped '
      'once the screen has left select', () {
    fakeAsync((async) {
      startAt0(async);
      server.ratesGate = Completer<void>();
      server.open = false;
      async.elapse(const Duration(seconds: 30));
      expect(server.ratesReads, 2);
      async.elapse(const Duration(seconds: 60));
      expect(server.ratesReads, 2, reason: 'the 30 s read is still in flight');

      fillUsd();
      c.activity();
      c.submitQuote();
      async.flushMicrotasks();
      expect(st().screen, KioskScreen.quote);

      server.ratesGate!.complete();
      server.ratesGate = null;
      async.flushMicrotasks();
      expect(st().screen, KioskScreen.quote);
      expect(st().error, '', reason: 'no closed banner over a quote');
      container.dispose();
    });
  });

  test('disposing the controller stops the refresh, even with a read in '
      'flight', () {
    fakeAsync((async) {
      startAt0(async);
      async.elapse(const Duration(seconds: 30));
      async.flushMicrotasks();
      expect(server.ratesReads, 2);

      server.ratesGate = Completer<void>();
      async.elapse(const Duration(seconds: 30));
      expect(server.ratesReads, 3);
      container.dispose();
      server.ratesGate!.complete();
      async.flushMicrotasks();

      async.elapse(const Duration(seconds: 90));
      expect(server.ratesReads, 3);
      expect(async.pendingTimers, isEmpty);
    });
  });
}
