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

class FakeRepository implements KioskRepository {
  final calls = <String>[];
  bool open = true;
  String serverStatus = 'QUOTED';
  String? receiptImage;
  String? transactionNo;
  String? failNext;
  Completer<RequestIdentity>? confirmGate;

  static const id = '05eb0fcb-d41f-465e-84b6-83a5d26c3ae6';

  Map<String, dynamic> get _identity => {
    'id': id,
    'queue': 'K-05EB0FCB',
    'orderNo': 'ORD-05EB0FCB',
    'transactionNo': transactionNo,
    'status': serverStatus,
  };

  void _enter(String action) {
    calls.add(action);
    final message = failNext;
    if (message != null) {
      failNext = null;
      throw KioskFailure(message);
    }
  }

  @override
  Future<RatesModel> rates() async {
    _enter('rates');
    return RatesModel.fromJson({
      'branchId': 'MAIN',
      'branchName': 'XC-B',
      'open': open,
      'rates': [
        {'code': 'USD', 'nameEn': 'US Dollar', 'buy': 32.1, 'sell': 32.6},
        {'code': 'JPY', 'nameEn': 'Japanese Yen', 'buy': 0.211, 'sell': 0.219},
      ],
    });
  }

  @override
  Future<QuoteModel> quote({
    required ExchangeType type,
    required String currency,
    required String amount,
    required String paymentMethod,
  }) async {
    _enter('quote');
    return QuoteModel.fromJson({
      ..._identity,
      'paymentMethod': paymentMethod,
      'type': type.wire,
      'currency': 'USD',
      'rate': 32.1,
      'foreign': 100,
      'thb': 3210,
    });
  }

  @override
  Future<RequestIdentity> confirm(String id) async {
    _enter('confirm');
    final gate = confirmGate;
    if (gate != null) {
      confirmGate = null;
      return gate.future;
    }
    serverStatus = 'WAITING';
    return RequestIdentity.fromJson(_identity);
  }

  @override
  Future<RequestIdentity> status(String id) async {
    _enter('status');
    return RequestIdentity.fromJson(_identity);
  }

  @override
  Future<ReceiptModel> customerReceipt(String id) async {
    _enter('customerReceipt');
    return ReceiptModel.fromJson({..._identity, 'image': receiptImage});
  }
}

KioskL10n l10n() => KioskL10n.fromJson({
  'langs': [
    {'code': 'en', 'lang': 'en', 'name': 'English'},
  ],
  'text': {
    'en': {
      'closed': 'closed-en',
      'errGeneric': 'generic-en',
      'errReview': 'review-en',
      'quoteExpired': 'expired-en',
      'statusError': 'status-error-en',
      'wait': 'Please review ({n})',
    },
  },
  'errors': {'Please review your quote for 3 seconds.': 'errReview'},
});

void main() {
  late FakeRepository server;
  late ProviderContainer container;
  late KioskController c;
  KioskState st() => container.read(kioskControllerProvider);

  KioskController make() {
    container = ProviderContainer(
      overrides: [
        kioskRepositoryProvider.overrideWithValue(server),
        kioskL10nProvider.overrideWithValue(l10n()),
        deviceLanguagesProvider.overrideWithValue(const ['en']),
      ],
    );
    container.listen(kioskControllerProvider, (_, _) {});
    return container.read(kioskControllerProvider.notifier);
  }

  void quoteUsd(FakeAsync async) {
    c.start();
    async.flushMicrotasks();
    c
      ..setCurrency('USD')
      ..setAmount('100')
      ..setPaymentMethod('Cash');
    c.submitQuote();
    async.flushMicrotasks();
  }

  setUp(() => server = FakeRepository());

  test('row 1: Confirm throws, server is at WAITING, next status poll moves '
      'to waiting, clears error, and starts receipt poller', () {
    fakeAsync((async) {
      c = make();
      quoteUsd(async);
      async.elapse(const Duration(seconds: 3));

      // Server commits WAITING, but the confirm reply is lost.
      server.serverStatus = 'WAITING';
      server.failNext = 'network timeout after commit';
      c.confirm();
      async.flushMicrotasks();
      expect(st().screen, KioskScreen.quote);
      expect(st().error, 'generic-en');

      // At t=5 s (2 s later), _statusPoller sees WAITING.
      async.elapse(const Duration(seconds: 2));
      expect(st().screen, KioskScreen.waiting);
      expect(st().error, '');
      expect(st().status!.status, KioskStatus.waiting);

      // Receipt poller is now running (ticks every 3 s).
      async.elapse(const Duration(seconds: 3));
      expect(server.calls, contains('customerReceipt'));
      container.dispose();
    });
  });

  test('row 2: Confirm throws, poll answers ACCEPTED or ID_CHECKED moves '
      'screen to waiting and shows that stage', () {
    fakeAsync((async) {
      c = make();
      quoteUsd(async);
      async.elapse(const Duration(seconds: 3));

      server.serverStatus = 'ACCEPTED';
      server.failNext = 'network timeout after commit';
      c.confirm();
      async.flushMicrotasks();

      async.elapse(const Duration(seconds: 2));
      expect(st().screen, KioskScreen.waiting);
      expect(st().error, '');
      expect(st().status!.status, KioskStatus.accepted);

      // Also verify ID_CHECKED directly from the quote screen.
      c.reset();
      async.flushMicrotasks();
      server.serverStatus = 'QUOTED';
      c
        ..setCurrency('USD')
        ..setAmount('100')
        ..setPaymentMethod('Cash');
      c.submitQuote();
      async.flushMicrotasks();
      async.elapse(const Duration(seconds: 3));

      server.serverStatus = 'ID_CHECKED';
      server.failNext = 'network timeout after commit';
      c.confirm();
      async.flushMicrotasks();

      async.elapse(const Duration(seconds: 2));
      expect(st().screen, KioskScreen.waiting);
      expect(st().error, '');
      expect(st().status!.status, KioskStatus.idChecked);
      container.dispose();
    });
  });

  test('row 3: Confirm throws, poll answers WAITING, then 2 idle minutes '
      'keeps screen at waiting', () {
    fakeAsync((async) {
      c = make();
      quoteUsd(async);
      async.elapse(const Duration(seconds: 3));

      server.serverStatus = 'WAITING';
      server.failNext = 'network timeout after commit';
      c.confirm();
      async.flushMicrotasks();

      async.elapse(const Duration(seconds: 2));
      expect(st().screen, KioskScreen.waiting);

      async.elapse(const Duration(minutes: 2));
      async.flushMicrotasks();
      expect(st().screen, KioskScreen.waiting);
      expect(st().quote, isNotNull);
      container.dispose();
    });
  });

  test(
    'row 4: Confirm refused ("review your quote for 3 seconds"), poll '
    'answers QUOTED stays quote with refusal text until WAITING is polled',
    () {
      fakeAsync((async) {
        c = make();
        quoteUsd(async);
        async.elapse(const Duration(seconds: 3));

        server.failNext = 'Please review your quote for 3 seconds.';
        c.confirm();
        async.flushMicrotasks();
        expect(st().screen, KioskScreen.quote);
        expect(st().error, 'review-en');

        // Status poll answers QUOTED: stays on quote with the refusal text.
        async.elapse(const Duration(seconds: 2));
        expect(st().screen, KioskScreen.quote);
        expect(st().error, 'review-en');

        // Next confirm commits WAITING on server but loses reply; poll recovers.
        server.serverStatus = 'WAITING';
        server.failNext = 'network timeout after commit';
        c.confirm();
        async.flushMicrotasks();
        async.elapse(const Duration(seconds: 5));
        expect(st().screen, KioskScreen.waiting);
        expect(st().error, '');
        container.dispose();
      });
    },
  );

  test('row 5: Poll answers EXPIRED on the quote screen sets quoteExpired and '
      'locks Confirm, whereas WAITING transitions to waiting', () {
    fakeAsync((async) {
      c = make();
      quoteUsd(async);
      server.serverStatus = 'EXPIRED';
      async.elapse(const Duration(seconds: 5));
      expect(st().quoteExpired, isTrue);
      expect(st().canConfirm, isFalse);
      expect(st().error, 'expired-en');

      // New quote where confirm reply is lost but status poll answers WAITING.
      c.backToSelect();
      server.serverStatus = 'QUOTED';
      c
        ..setCurrency('USD')
        ..setAmount('100')
        ..setPaymentMethod('Cash');
      c.submitQuote();
      async.flushMicrotasks();
      async.elapse(const Duration(seconds: 3));
      server.serverStatus = 'WAITING';
      server.failNext = 'network timeout after commit';
      c.confirm();
      async.flushMicrotasks();
      async.elapse(const Duration(seconds: 2));
      expect(st().screen, KioskScreen.waiting);
      expect(st().quoteExpired, isFalse);
      expect(st().error, '');
      container.dispose();
    });
  });

  test('row 6: Confirm succeeds moves to waiting, and a lost confirm reply '
      'reaches the same waiting state on the next status poll', () {
    fakeAsync((async) {
      c = make();
      quoteUsd(async);
      async.elapse(const Duration(seconds: 3));
      c.confirm();
      async.flushMicrotasks();
      expect(st().screen, KioskScreen.waiting);
      expect(st().status!.status, KioskStatus.waiting);

      c.reset();
      async.flushMicrotasks();
      server.serverStatus = 'QUOTED';
      c
        ..setCurrency('USD')
        ..setAmount('100')
        ..setPaymentMethod('Cash');
      c.submitQuote();
      async.flushMicrotasks();
      async.elapse(const Duration(seconds: 3));
      server.serverStatus = 'WAITING';
      server.failNext = 'network timeout after commit';
      c.confirm();
      async.flushMicrotasks();
      async.elapse(const Duration(seconds: 2));
      expect(st().screen, KioskScreen.waiting);
      expect(st().status!.status, KioskStatus.waiting);
      expect(st().error, '');
      container.dispose();
    });
  });

  test('edge case: poll answers COMPLETED directly on quote screen moves to '
      'waiting and schedules completion restart', () {
    fakeAsync((async) {
      c = make();
      quoteUsd(async);
      async.elapse(const Duration(seconds: 3));

      server
        ..serverStatus = 'COMPLETED'
        ..transactionNo = 'XC-20261016-000001'
        ..failNext = 'network timeout after commit';
      c.confirm();
      async.flushMicrotasks();

      async.elapse(const Duration(seconds: 2));
      expect(st().screen, KioskScreen.waiting);
      expect(st().error, '');
      expect(st().status!.status, KioskStatus.completed);

      async.elapse(const Duration(seconds: 29));
      expect(st().screen, KioskScreen.waiting);
      async.elapse(const Duration(seconds: 1));
      async.flushMicrotasks();
      expect(st().screen, KioskScreen.select);
      expect(st().quote, isNull);
      container.dispose();
    });
  });

  test('in-flight confirm throws after status poll already moved screen to '
      'waiting: keeps screen waiting and error empty', () {
    fakeAsync((async) {
      c = make();
      quoteUsd(async);
      async.elapse(const Duration(seconds: 3));

      final gate = Completer<RequestIdentity>();
      server
        ..confirmGate = gate
        ..serverStatus = 'WAITING';
      c.confirm();
      async.flushMicrotasks();
      expect(st().busy, isTrue);
      expect(st().screen, KioskScreen.quote);

      // At +2 s (t=5 s), status poll sees WAITING while confirm() is still in flight.
      async.elapse(const Duration(seconds: 2));
      expect(st().screen, KioskScreen.waiting);
      expect(st().error, '');

      // Now confirm() fails (e.g. receiveTimeout after server committed).
      gate.completeError(const KioskFailure('receiveTimeout'));
      async.flushMicrotasks();
      expect(st().screen, KioskScreen.waiting);
      expect(st().error, '');
      expect(st().busy, isFalse);
      container.dispose();
    });
  });
}
