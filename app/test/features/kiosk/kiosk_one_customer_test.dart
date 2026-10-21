// #4243: the kiosk is shared, one customer at a time. After confirm it shows
// the queue number with a one-press ยกเลิกคิว and เสร็จ for N seconds, then
// starts over; idle resets every screen. Written before the implementation by
// the sub-controller claude/36d9dc6a. Do not edit, delete or weaken these
// tests; add tests in a file of your own.
//
// What these tests pin (#4243 §3, §4):
// - `KioskController.resultSeconds` (static const int, 30) is the default N.
// - `kioskResultSecondsProvider` (a `Provider<int>` declared in
//   kiosk_controller.dart, defaulting to `KioskController.resultSeconds`) is
//   the one place the controller reads N from. A later ticket overrides it
//   with the branch's kiosk setting.
// - N seconds after a successful confirm, the kiosk resets to
//   `KioskScreen.select`: quote, status and the cancel token are gone.
// - `KioskController.done()` resets at once. `WaitingPage` shows it as a
//   button with `Key('done')`.
// - `Key('cancel-queue')` calls `customerCancel` once with the token and opens
//   no dialog. The cancelled message shows, then the kiosk resets N seconds
//   after the cancel.
// - The idle timeout (2 min) resets the waiting screen too: there is no
//   `atCounter` exemption.
// - After a reset, nothing polls the old request.
import 'package:fake_async/fake_async.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/features/kiosk/app/kiosk_providers.dart';
import 'package:xchange_kiosk/features/kiosk/domain/entities/kiosk_models.dart';
import 'package:xchange_kiosk/features/kiosk/domain/repository/kiosk_repository.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/controllers/kiosk_controller.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/pages/waiting_page.dart';
import 'package:xchange_kiosk/l10n/kiosk_l10n.dart';

class _Server implements KioskRepository {
  final calls = <String>[];
  String serverStatus = 'QUOTED';
  int cancels = 0;
  String? lastToken;

  static const id = 'k-one-1';

  Map<String, dynamic> _identity({String? cancelToken}) => {
    'id': id,
    'queue': 'K042',
    'orderNo': 'ORD-K042',
    'transactionNo': null,
    'status': serverStatus,
    'cancelToken': ?cancelToken,
  };

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
    return QuoteModel.fromJson({
      ..._identity(),
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
    return RequestIdentity.fromJson(_identity(cancelToken: 'tok-one'));
  }

  @override
  Future<RequestIdentity> status(String id) async {
    calls.add('status');
    return RequestIdentity.fromJson(_identity());
  }

  @override
  Future<ReceiptModel> customerReceipt(String id) async {
    calls.add('customerReceipt');
    return ReceiptModel.fromJson({..._identity(), 'image': null});
  }

  // Declared without @override so this file compiles both before and after
  // customerCancel moves onto the KioskRepository interface (#4236).
  Future<RequestIdentity> customerCancel(String id, String cancelToken) async {
    calls.add('customerCancel');
    cancels += 1;
    lastToken = cancelToken;
    serverStatus = 'CANCELLED';
    return RequestIdentity.fromJson(_identity());
  }

  /// Calls about the request (everything but the rates refresh).
  List<String> get requestCalls => calls.where((c) => c != 'rates').toList();
}

KioskL10n _l10n() => KioskL10n.fromJson({
  'langs': [
    {'code': 'en', 'lang': 'en', 'name': 'English'},
  ],
  'text': {
    'en': {
      'sentTitle': 'Your request has been sent',
      'orderNo': 'Order no.',
      'transactionNo': 'Transaction no.',
      'notCreated': 'Not created yet',
      'status': 'Status',
      'stWAITING': 'Waiting for staff',
      'stCANCELLED': 'Cancelled',
      'bring': 'Bring passport',
      'requestOnly': 'Request only',
      'stepId': 'Checking ID',
      'stepPending': 'Pending',
      'stepSuccess': 'Success',
      'phaseWaiting': 'Waiting for staff',
      'phasePassport': 'Please show your passport',
      'summaryTitle': 'Order summary',
      'youGive': 'You give',
      'payment': 'Payment method',
      'cash': 'Cash',
      'rate': 'Exchange rate',
      'youReceive': 'You receive',
      'summaryNote': 'Summary note',
      'usdtTitle': 'USDT',
      'usdtScan': 'Scan',
      'usdtNote': 'Note',
      'cancelQueue': 'Cancel queue',
      'done': 'Done',
      'requestCancelled':
          'Your request has been cancelled. Please contact our staff.',
      'errGeneric': 'generic-en',
    },
  },
  'errors': <String, String>{},
});

ProviderContainer _container(_Server server, {int? resultSeconds}) {
  final container = ProviderContainer(
    overrides: [
      kioskRepositoryProvider.overrideWithValue(server),
      kioskL10nProvider.overrideWithValue(_l10n()),
      deviceLanguagesProvider.overrideWithValue(const ['en']),
      if (resultSeconds != null)
        kioskResultSecondsProvider.overrideWithValue(resultSeconds),
    ],
  );
  container.listen(kioskControllerProvider, (_, _) {});
  return container;
}

/// Quote, wait out the 3 s review, confirm: the kiosk is on the waiting screen.
void _confirmIn(FakeAsync async, ProviderContainer container) {
  final c = container.read(kioskControllerProvider.notifier);
  c.start();
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
}

void _expectStartedOver(ProviderContainer container) {
  final s = container.read(kioskControllerProvider);
  expect(s.screen, KioskScreen.select, reason: 'back on the start screen');
  expect(s.quote, isNull, reason: 'the old quote is gone');
  expect(s.status, isNull, reason: 'the old request is gone');
  expect(s.canCancel, isFalse, reason: 'the old cancel token is gone');
}

void main() {
  test('the default result window is 30 s, read from one place', () {
    expect(KioskController.resultSeconds, 30);
    final container = _container(_Server());
    expect(container.read(kioskResultSecondsProvider), 30);
    container.dispose();
  });

  test(
    'row 1: after confirm the queue number and ยกเลิกคิว show, and at 30 s the kiosk starts over',
    () {
      fakeAsync((async) {
        final server = _Server();
        final container = _container(server);
        _confirmIn(async, container);

        final s = container.read(kioskControllerProvider);
        expect(s.screen, KioskScreen.waiting);
        expect(s.identity?.queue, 'K042');
        expect(s.canCancel, isTrue);

        async.elapse(const Duration(seconds: 29));
        async.flushMicrotasks();
        expect(
          container.read(kioskControllerProvider).screen,
          KioskScreen.waiting,
          reason: 'still showing the queue number at 29 s',
        );

        async.elapse(const Duration(seconds: 1));
        async.flushMicrotasks();
        _expectStartedOver(container);
        container.dispose();
      });
    },
  );

  test(
    'row 4 (with rows 1 and 3): resultSeconds = 10 makes both windows 10 s',
    () {
      fakeAsync((async) {
        final server = _Server();
        final container = _container(server, resultSeconds: 10);
        _confirmIn(async, container);
        async.elapse(const Duration(seconds: 10));
        async.flushMicrotasks();
        _expectStartedOver(container);

        // Row 3's window: N seconds after the cancel.
        _confirmIn(async, container);
        container.read(kioskControllerProvider.notifier).cancel();
        async.flushMicrotasks();
        expect(
          container.read(kioskControllerProvider).status?.status,
          KioskStatus.cancelled,
        );
        async.elapse(const Duration(seconds: 9));
        async.flushMicrotasks();
        expect(
          container.read(kioskControllerProvider).screen,
          KioskScreen.waiting,
          reason: 'the cancelled message stays for N seconds',
        );
        async.elapse(const Duration(seconds: 1));
        async.flushMicrotasks();
        _expectStartedOver(container);
        container.dispose();
      });
    },
  );

  test(
    'row 3: a cancel keeps the cancelled message for N = 30 s, then starts over',
    () {
      fakeAsync((async) {
        final server = _Server();
        final container = _container(server);
        _confirmIn(async, container);
        async.elapse(const Duration(seconds: 5));
        async.flushMicrotasks();

        container.read(kioskControllerProvider.notifier).cancel();
        async.flushMicrotasks();
        expect(server.cancels, 1);
        expect(server.lastToken, 'tok-one');
        expect(
          container.read(kioskControllerProvider).error,
          'Your request has been cancelled. Please contact our staff.',
        );

        async.elapse(const Duration(seconds: 29));
        async.flushMicrotasks();
        expect(
          container.read(kioskControllerProvider).screen,
          KioskScreen.waiting,
          reason:
              'the window restarts at the cancel: 29 s after it, still shown',
        );
        async.elapse(const Duration(seconds: 1));
        async.flushMicrotasks();
        _expectStartedOver(container);
        container.dispose();
      });
    },
  );

  test('row 2: done() starts over at once', () {
    fakeAsync((async) {
      final server = _Server();
      final container = _container(server);
      _confirmIn(async, container);
      container.read(kioskControllerProvider.notifier).done();
      async.flushMicrotasks();
      _expectStartedOver(container);
      container.dispose();
    });
  });

  test(
    'row 5: idle on the waiting screen starts over (no atCounter exemption)',
    () {
      fakeAsync((async) {
        final server = _Server();
        // A result window longer than the 2 min idle timeout, so only idle can reset.
        final container = _container(server, resultSeconds: 600);
        _confirmIn(async, container);
        expect(
          container.read(kioskControllerProvider).screen,
          KioskScreen.waiting,
        );

        async.elapse(const Duration(minutes: 2, seconds: 1));
        async.flushMicrotasks();
        _expectStartedOver(container);
        container.dispose();
      });
    },
  );

  test('row 6: after a reset nothing polls the old request', () {
    fakeAsync((async) {
      final server = _Server();
      final container = _container(server);
      _confirmIn(async, container);
      async.elapse(const Duration(seconds: 30));
      async.flushMicrotasks();
      _expectStartedOver(container);

      final atReset = server.requestCalls;
      async.elapse(const Duration(minutes: 1));
      async.flushMicrotasks();
      expect(
        server.requestCalls,
        atReset,
        reason: 'no status or receipt poll for the old request',
      );
      container.dispose();
    });
  });

  group('the waiting page', () {
    Future<ProviderContainer> pumpWaiting(
      WidgetTester tester,
      _Server server,
    ) async {
      final container = _container(server);
      final c = container.read(kioskControllerProvider.notifier);
      await c.start();
      c
        ..setCurrency('USD')
        ..setAmount('100')
        ..setPaymentMethod('Cash');
      await c.submitQuote();
      await tester.pump(const Duration(seconds: 3));
      await c.confirm();
      await tester.pumpWidget(
        UncontrolledProviderScope(
          container: container,
          child: const MaterialApp(
            home: Scaffold(body: SingleChildScrollView(child: WaitingPage())),
          ),
        ),
      );
      await tester.pump();
      return container;
    }

    testWidgets('row 2: the เสร็จ button (Key done) starts over at once', (
      tester,
    ) async {
      final server = _Server();
      final container = await pumpWaiting(tester, server);
      expect(find.byKey(const Key('done')), findsOneWidget);
      await tester.ensureVisible(find.byKey(const Key('done')));
      await tester.tap(find.byKey(const Key('done')));
      await tester.pump();
      _expectStartedOver(container);
      await tester.pumpWidget(const SizedBox.shrink());
      container.dispose();
    });

    testWidgets('row 3: one press of ยกเลิกคิว cancels, with no dialog', (
      tester,
    ) async {
      final server = _Server();
      final container = await pumpWaiting(tester, server);
      await tester.ensureVisible(find.byKey(const Key('cancel-queue')));
      await tester.tap(find.byKey(const Key('cancel-queue')));
      await tester.pump();
      expect(find.byType(Dialog), findsNothing);
      expect(find.byType(AlertDialog), findsNothing);
      expect(server.cancels, 1);
      expect(server.lastToken, 'tok-one');
      expect(
        container.read(kioskControllerProvider).status?.status,
        KioskStatus.cancelled,
      );
      await tester.pumpWidget(const SizedBox.shrink());
      container.dispose();
    });
  });
}
