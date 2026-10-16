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

class GateRepository implements KioskRepository {
  final calls = <String>[];
  final confirmGates = <Completer<RequestIdentity>>[];
  final statusGates = <Completer<RequestIdentity>>[];
  final receiptGates = <Completer<ReceiptModel>>[];
  String? failNextConfirm;
  int quoteSeq = 0;

  static const defaultId = '05eb0fcb-d41f-465e-84b6-83a5d26c3ae6';

  static RequestIdentity identity(
    String status, {
    String id = defaultId,
    String? transactionNo,
  }) => RequestIdentity.fromJson({
    'id': id,
    'queue': 'K-05EB0FCB',
    'orderNo': 'ORD-05EB0FCB',
    'transactionNo': transactionNo,
    'status': status,
  });

  static ReceiptModel receipt(
    String status, {
    String id = defaultId,
    String? image,
    String? transactionNo,
  }) => ReceiptModel.fromJson({
    'id': id,
    'queue': 'K-05EB0FCB',
    'orderNo': 'ORD-05EB0FCB',
    'transactionNo': transactionNo,
    'status': status,
    'image': image,
  });

  @override
  Future<RatesModel> rates() async {
    calls.add('rates');
    return RatesModel.fromJson({
      'branchId': 'MAIN',
      'branchName': 'XC-B',
      'open': true,
      'rates': [
        {'code': 'USD', 'nameEn': 'US Dollar', 'buy': 32.1, 'sell': 32.6},
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
    calls.add('quote');
    quoteSeq++;
    final id = quoteSeq == 1 ? defaultId : '$defaultId-$quoteSeq';
    return QuoteModel.fromJson({
      'id': id,
      'queue': 'K-05EB0FCB',
      'orderNo': 'ORD-05EB0FCB',
      'transactionNo': null,
      'status': 'QUOTED',
      'paymentMethod': paymentMethod,
      'type': type.wire,
      'currency': currency,
      'rate': 32.1,
      'foreign': 100,
      'thb': 3210,
    });
  }

  @override
  Future<RequestIdentity> confirm(String id) {
    calls.add('confirm');
    final err = failNextConfirm;
    if (err != null) {
      failNextConfirm = null;
      return Future<RequestIdentity>.error(KioskFailure(err));
    }
    final gate = Completer<RequestIdentity>();
    confirmGates.add(gate);
    return gate.future;
  }

  @override
  Future<RequestIdentity> status(String id) {
    calls.add('status');
    final gate = Completer<RequestIdentity>();
    statusGates.add(gate);
    return gate.future;
  }

  @override
  Future<ReceiptModel> customerReceipt(String id) {
    calls.add('customerReceipt');
    final gate = Completer<ReceiptModel>();
    receiptGates.add(gate);
    return gate.future;
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
  late GateRepository server;
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

  setUp(() => server = GateRepository());

  test('row 1: receipt read sets COMPLETED, then a late status read answers '
      'ACCEPTED — status stays COMPLETED', () {
    fakeAsync((async) {
      c = make();
      quoteUsd(async);
      async.elapse(const Duration(seconds: 3));

      c.confirm();
      async.flushMicrotasks();
      server.confirmGates.removeLast().complete(
        GateRepository.identity('WAITING'),
      );
      async.flushMicrotasks();
      expect(st().screen, KioskScreen.waiting);
      expect(st().status!.status, KioskStatus.waiting);

      // At t=6 s (3 s after confirm), status poll #1 (t=5 s) and receipt
      // poll #1 (t=6 s) are both in flight.
      async.elapse(const Duration(seconds: 3));
      expect(server.statusGates, isNotEmpty);
      expect(server.receiptGates, isNotEmpty);
      final statusPoll = server.statusGates.removeLast();
      final receiptPoll = server.receiptGates.removeLast();

      // Receipt read finishes first with COMPLETED.
      receiptPoll.complete(
        GateRepository.receipt(
          'COMPLETED',
          transactionNo: 'XC-20261016-000001',
        ),
      );
      async.flushMicrotasks();
      expect(st().status!.status, KioskStatus.completed);

      // Earlier status read answers late with ACCEPTED: status must stay COMPLETED.
      statusPoll.complete(GateRepository.identity('ACCEPTED'));
      async.flushMicrotasks();
      expect(st().status!.status, KioskStatus.completed);
      container.dispose();
    });
  });

  test('row 2: status poll moved to waiting with ACCEPTED, then a late confirm '
      'success answers WAITING — status stays ACCEPTED, busy false', () {
    fakeAsync((async) {
      c = make();
      quoteUsd(async);
      async.elapse(const Duration(seconds: 3));

      c.confirm();
      async.flushMicrotasks();
      expect(st().busy, isTrue);
      final slowConfirm = server.confirmGates.removeLast();

      // At t=5 s (+2 s), status poll fires and answers ACCEPTED while confirm
      // is still in flight.
      async.elapse(const Duration(seconds: 2));
      server.statusGates.removeLast().complete(
        GateRepository.identity('ACCEPTED'),
      );
      async.flushMicrotasks();
      expect(st().screen, KioskScreen.waiting);
      expect(st().status!.status, KioskStatus.accepted);

      // Late confirm success arrives with older WAITING status.
      slowConfirm.complete(GateRepository.identity('WAITING'));
      async.flushMicrotasks();
      expect(st().screen, KioskScreen.waiting);
      expect(st().status!.status, KioskStatus.accepted);
      expect(st().busy, isFalse);
      container.dispose();
    });
  });

  test('row 3: screen is on receipt (or left quote), a late confirm success '
      'arrives — screen stays on receipt and ignores stale quote confirm', () {
    fakeAsync((async) {
      c = make();
      quoteUsd(async);
      async.elapse(const Duration(seconds: 3));

      c.confirm();
      async.flushMicrotasks();
      final slowConfirm = server.confirmGates.removeLast();

      // Status poll moves screen to waiting.
      async.elapse(const Duration(seconds: 2));
      server.statusGates.removeLast().complete(
        GateRepository.identity('ID_CHECKED'),
      );
      async.flushMicrotasks();
      expect(st().screen, KioskScreen.waiting);

      // Receipt poll delivers the receipt image, moving screen to receipt.
      async.elapse(const Duration(seconds: 3));
      server.receiptGates.removeLast().complete(
        GateRepository.receipt(
          'COMPLETED',
          image: 'data:image/png;base64,AAA',
          transactionNo: 'XC-20261016-000002',
        ),
      );
      async.flushMicrotasks();
      expect(st().screen, KioskScreen.receipt);
      expect(st().status!.status, KioskStatus.completed);

      // Late confirm success arrives: screen must stay on receipt and status COMPLETED.
      slowConfirm.complete(GateRepository.identity('WAITING'));
      async.flushMicrotasks();
      expect(st().screen, KioskScreen.receipt);
      expect(st().status!.status, KioskStatus.completed);
      expect(st().busy, isFalse);

      // Also verify: if screen already left the quote (reset/backToSelect),
      // a late confirm reply is ignored.
      c.reset();
      async.flushMicrotasks();
      c
        ..setCurrency('USD')
        ..setAmount('100')
        ..setPaymentMethod('Cash');
      c.submitQuote();
      async.flushMicrotasks();
      async.elapse(const Duration(seconds: 3));
      c.confirm();
      async.flushMicrotasks();
      final abandonedConfirm = server.confirmGates.removeLast();
      c.backToSelect();
      expect(st().screen, KioskScreen.select);

      abandonedConfirm.complete(
        GateRepository.identity('WAITING', id: '${GateRepository.defaultId}-2'),
      );
      async.flushMicrotasks();
      expect(st().screen, KioskScreen.select);
      expect(st().busy, isFalse);
      container.dispose();
    });
  });

  test('row 4: normal forward progress (waiting → accepted → idChecked → '
      'completed) shows each step and ignores a late receipt answer', () {
    fakeAsync((async) {
      c = make();
      quoteUsd(async);
      async.elapse(const Duration(seconds: 3));

      // 1. waiting
      c.confirm();
      async.flushMicrotasks();
      server.confirmGates.removeLast().complete(
        GateRepository.identity('WAITING'),
      );
      async.flushMicrotasks();
      expect(st().status!.status, KioskStatus.waiting);

      // 2. accepted (from status poll at t=5 s), while receipt poll #1 (t=6 s) is held
      async.elapse(const Duration(seconds: 3));
      final oldReceiptPoll = server.receiptGates.removeLast();
      server.statusGates.removeLast().complete(
        GateRepository.identity('ACCEPTED'),
      );
      async.flushMicrotasks();
      expect(st().status!.status, KioskStatus.accepted);

      // 3. idChecked (from status poll at t=10 s)
      async.elapse(const Duration(seconds: 5));
      server.statusGates.removeLast().complete(
        GateRepository.identity('ID_CHECKED'),
      );
      async.flushMicrotasks();
      expect(st().status!.status, KioskStatus.idChecked);

      // Late receipt poll #1 answers with older ACCEPTED: stays idChecked.
      oldReceiptReceiptComplete(oldReceiptPoll, 'ACCEPTED');
      async.flushMicrotasks();
      expect(st().status!.status, KioskStatus.idChecked);

      // 4. completed
      async.elapse(const Duration(seconds: 3));
      server.receiptGates.removeLast().complete(
        GateRepository.receipt(
          'COMPLETED',
          image: 'data:image/png;base64,BBB',
          transactionNo: 'XC-20261016-000003',
        ),
      );
      async.flushMicrotasks();
      expect(st().status!.status, KioskStatus.completed);
      expect(st().screen, KioskScreen.receipt);
      container.dispose();
    });
  });

  test('row 5: EXPIRED keeps quoteExpired handling, and lost/late confirm '
      'replies preserve newer poll status', () {
    fakeAsync((async) {
      c = make();
      quoteUsd(async);

      // First status poll on quote screen answers QUOTED, then next answers EXPIRED.
      async.elapse(const Duration(seconds: 5));
      server.statusGates.removeLast().complete(
        GateRepository.identity('QUOTED'),
      );
      async.flushMicrotasks();
      expect(st().status!.status, KioskStatus.quoted);

      async.elapse(const Duration(seconds: 5));
      server.statusGates.removeLast().complete(
        GateRepository.identity('EXPIRED'),
      );
      async.flushMicrotasks();
      expect(st().status!.status, KioskStatus.expired);
      expect(st().quoteExpired, isTrue);
      expect(st().canConfirm, isFalse);
      expect(st().error, 'expired-en');

      // Start a new quote: first confirm fails (lost reply), poll recovers to
      // ACCEPTED, and a late second confirm answering WAITING does not step back.
      c.backToSelect();
      c
        ..setCurrency('USD')
        ..setAmount('100')
        ..setPaymentMethod('Cash');
      c.submitQuote();
      async.flushMicrotasks();
      final secondId = '${GateRepository.defaultId}-2';
      async.elapse(const Duration(seconds: 3));

      server.failNextConfirm = 'network timeout after commit';
      c.confirm();
      async.flushMicrotasks();
      expect(st().screen, KioskScreen.quote);
      expect(st().error, 'generic-en');

      // Customer taps confirm again while status poll is about to answer ACCEPTED.
      c.confirm();
      async.flushMicrotasks();
      final retryConfirm = server.confirmGates.removeLast();

      async.elapse(const Duration(seconds: 2));
      server.statusGates.removeLast().complete(
        GateRepository.identity('ACCEPTED', id: secondId),
      );
      async.flushMicrotasks();
      expect(st().screen, KioskScreen.waiting);
      expect(st().error, '');
      expect(st().status!.status, KioskStatus.accepted);

      retryConfirm.complete(GateRepository.identity('WAITING', id: secondId));
      async.flushMicrotasks();
      expect(st().screen, KioskScreen.waiting);
      expect(st().status!.status, KioskStatus.accepted);
      expect(st().busy, isFalse);
      container.dispose();
    });
  });
}

void oldReceiptReceiptComplete(Completer<ReceiptModel> gate, String status) {
  gate.complete(GateRepository.receipt(status));
}
