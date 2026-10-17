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

  test('confirm refused, then a status poll succeeds (not EXPIRED) keeps the '
      'refusal text', () {
    fakeAsync((async) {
      c = make();
      quoteUsd(async);
      async.elapse(const Duration(seconds: 3));
      server.failNext = 'Please review your quote for 3 seconds.';
      c.confirm();
      async.flushMicrotasks();
      expect(st().error, 'review-en');

      // 2 s later (5 s after submitQuote), _statusPoller runs a successful poll.
      async.elapse(const Duration(seconds: 2));
      expect(server.calls.last, 'status');
      expect(st().error, 'review-en');
      container.dispose();
    });
  });

  test(
    'reportError(\'errGeneric\'), then a status poll succeeds keeps the text',
    () {
      fakeAsync((async) {
        c = make();
        quoteUsd(async);
        c.reportError('errGeneric');
        expect(st().error, 'generic-en');

        async.elapse(const Duration(seconds: 5));
        expect(server.calls.last, 'status');
        expect(st().error, 'generic-en');
        container.dispose();
      });
    },
  );

  test(
    'a poll failed (statusError shown), then a poll succeeds clears statusError',
    () {
      fakeAsync((async) {
        c = make();
        quoteUsd(async);
        server.failNext = 'poll failed';
        async.elapse(const Duration(seconds: 5));
        expect(st().error, 'status-error-en');

        async.elapse(const Duration(seconds: 5));
        expect(st().error, '');

        // Only statusError was cleared; a non-poll error still survives a poll.
        c.reportError('errGeneric');
        async.elapse(const Duration(seconds: 5));
        expect(st().error, 'generic-en');
        container.dispose();
      });
    },
  );

  test(
    'a poll answers EXPIRED on the quote screen sets the quoteExpired text',
    () {
      fakeAsync((async) {
        c = make();
        quoteUsd(async);
        async.elapse(const Duration(seconds: 3));
        server.failNext = 'Please review your quote for 3 seconds.';
        c.confirm();
        async.flushMicrotasks();
        expect(st().error, 'review-en');

        // Non-expired poll at t=5 s keeps the refusal error.
        async.elapse(const Duration(seconds: 2));
        expect(st().error, 'review-en');

        // Expired poll at t=10 s replaces it with quoteExpired.
        server.serverStatus = 'EXPIRED';
        async.elapse(const Duration(seconds: 5));
        expect(st().quoteExpired, isTrue);
        expect(st().error, 'expired-en');
        container.dispose();
      });
    },
  );

  test('confirm refused, then the customer taps Confirm again clears error at '
      'the start of that call', () {
    fakeAsync((async) {
      c = make();
      quoteUsd(async);
      async.elapse(const Duration(seconds: 3));
      server.failNext = 'Please review your quote for 3 seconds.';
      c.confirm();
      async.flushMicrotasks();
      expect(st().error, 'review-en');

      // Status poll at t=5 s does not clear the refusal before the retry.
      async.elapse(const Duration(seconds: 2));
      expect(st().error, 'review-en');

      final gate = Completer<RequestIdentity>();
      server.confirmGate = gate;
      c.confirm();
      expect(st().busy, isTrue);
      expect(st().error, '');

      server.serverStatus = 'WAITING';
      gate.complete(
        RequestIdentity.fromJson({
          'id': FakeRepository.id,
          'queue': 'K-05EB0FCB',
          'orderNo': 'ORD-05EB0FCB',
          'transactionNo': null,
          'status': 'WAITING',
        }),
      );
      async.flushMicrotasks();
      expect(st().screen, KioskScreen.waiting);
      container.dispose();
    });
  });
}
