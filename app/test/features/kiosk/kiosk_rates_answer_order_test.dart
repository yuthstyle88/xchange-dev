import 'dart:async';

import 'package:fake_async/fake_async.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/features/kiosk/app/kiosk_providers.dart';
import 'package:xchange_kiosk/features/kiosk/domain/entities/kiosk_models.dart';
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
  Completer<void>? nextRatesGate;
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
    final gate = nextRatesGate;
    nextRatesGate = null;
    final answer = {
      'branchId': 'MAIN',
      'branchName': 'XC-B',
      'open': open,
      'rates': [...rows],
    };
    if (gate != null) await gate.future;
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

  test('row 1: Refresh in flight (open), reset() answers closed, then the old '
      'refresh lands -> Error closed, model.open false', () {
    fakeAsync((async) {
      startAt0(async);
      fillUsd();
      expect(st().model!.open, true);
      expect(st().error, '');

      final slowRefresh = Completer<void>();
      server.nextRatesGate = slowRefresh;
      server.open = true;
      async.elapse(const Duration(seconds: 30));
      expect(server.ratesReads, 2);

      server.open = false;
      c.reset();
      async.flushMicrotasks();
      expect(server.ratesReads, 3);
      expect(st().model!.open, false);
      expect(st().error, 'closed-en');

      slowRefresh.complete();
      async.flushMicrotasks();

      expect(st().model!.open, false);
      expect(st().error, 'closed-en');
      expect(st().canReview, false);
      container.dispose();
    });
  });

  test('row 2: Refresh in flight with old rates, reset() answers with '
      'different rates, then the old refresh lands -> The reset\'s rates '
      'are kept', () {
    fakeAsync((async) {
      startAt0(async);
      expect(codes(), ['USD', 'JPY']);

      final slowRefresh = Completer<void>();
      server.nextRatesGate = slowRefresh;
      server.rows = [_usd, _jpy];
      async.elapse(const Duration(seconds: 30));
      expect(server.ratesReads, 2);

      server.rows = [_eur];
      c.reset();
      async.flushMicrotasks();
      expect(server.ratesReads, 3);
      expect(codes(), ['EUR']);
      expect(st().currency, 'EUR');

      slowRefresh.complete();
      async.flushMicrotasks();

      expect(codes(), ['EUR']);
      expect(st().currency, 'EUR');
      container.dispose();
    });
  });

  test(
    'row 3: Refresh in flight when the screen leaves select is dropped, '
    'and a refresh in flight before reset() is also dropped when it lands',
    () {
      fakeAsync((async) {
        startAt0(async);
        final quoteRefresh = Completer<void>();
        server.nextRatesGate = quoteRefresh;
        server.open = false;
        async.elapse(const Duration(seconds: 30));
        expect(server.ratesReads, 2);

        fillUsd();
        c.activity();
        c.submitQuote();
        async.flushMicrotasks();
        expect(st().screen, KioskScreen.quote);

        quoteRefresh.complete();
        async.flushMicrotasks();
        expect(st().screen, KioskScreen.quote);
        expect(st().error, '');

        server.open = true;
        c.reset();
        async.flushMicrotasks();
        expect(st().screen, KioskScreen.select);
        expect(st().model!.open, true);

        final slowRefresh = Completer<void>();
        server.nextRatesGate = slowRefresh;
        server.open = true;
        async.elapse(const Duration(seconds: 30));

        server.open = false;
        c.reset();
        async.flushMicrotasks();
        expect(st().model!.open, false);
        expect(st().error, 'closed-en');

        slowRefresh.complete();
        async.flushMicrotasks();
        expect(st().model!.open, false);
        expect(st().error, 'closed-en');
        container.dispose();
      });
    },
  );

  test('row 4: Dispose stops the refresh, and a stale refresh before reset() '
      'does not overwrite the reset state before dispose', () {
    fakeAsync((async) {
      startAt0(async);

      final staleRefresh = Completer<void>();
      server.nextRatesGate = staleRefresh;
      server.open = true;
      async.elapse(const Duration(seconds: 30));

      server.open = false;
      c.reset();
      async.flushMicrotasks();
      expect(st().model!.open, false);
      expect(st().error, 'closed-en');

      staleRefresh.complete();
      async.flushMicrotasks();
      expect(st().model!.open, false);
      expect(st().error, 'closed-en');

      final disposeRefresh = Completer<void>();
      server.nextRatesGate = disposeRefresh;
      async.elapse(const Duration(seconds: 30));
      final readsAtDispose = server.ratesReads;
      container.dispose();
      disposeRefresh.complete();
      async.flushMicrotasks();

      async.elapse(const Duration(seconds: 90));
      expect(server.ratesReads, readsAtDispose);
      expect(async.pendingTimers, isEmpty);
    });
  });

  test('edge case: Two resets in a row -> only the newer reset\'s answer '
      'applies', () {
    fakeAsync((async) {
      startAt0(async);

      final firstReset = Completer<void>();
      server.nextRatesGate = firstReset;
      server.open = true;
      server.rows = [_usd, _jpy];
      c.reset();
      expect(server.ratesReads, 2);

      server.open = false;
      server.rows = [_eur];
      c.reset();
      async.flushMicrotasks();
      expect(server.ratesReads, 3);
      expect(st().model!.open, false);
      expect(st().error, 'closed-en');
      expect(codes(), ['EUR']);
      expect(st().currency, 'EUR');

      firstReset.complete();
      async.flushMicrotasks();

      expect(st().model!.open, false);
      expect(st().error, 'closed-en');
      expect(codes(), ['EUR']);
      expect(st().currency, 'EUR');
      container.dispose();
    });
  });
}
