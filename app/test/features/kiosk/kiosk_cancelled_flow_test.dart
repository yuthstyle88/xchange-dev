import 'package:fake_async/fake_async.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/features/kiosk/app/kiosk_providers.dart';
import 'package:xchange_kiosk/features/kiosk/domain/entities/kiosk_models.dart';
import 'package:xchange_kiosk/features/kiosk/domain/kiosk_failure.dart';
import 'package:xchange_kiosk/features/kiosk/domain/repository/kiosk_repository.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/controllers/kiosk_controller.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/pages/waiting_page.dart';
import 'package:xchange_kiosk/l10n/kiosk_l10n.dart';

class _CancelFakeRepository implements KioskRepository {
  final calls = <String>[];
  String serverStatus = 'QUOTED';
  String? confirmCancelToken = 'tok-secret-99';
  String? lastCancelledId;
  String? lastCancelledToken;
  String? failCancel;

  static const id = 'k-cancel-1';

  Map<String, dynamic> _identity({String? cancelToken}) => {
    'id': id,
    'queue': 'K009',
    'orderNo': 'ORD-K009',
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
    return RequestIdentity.fromJson(_identity(cancelToken: confirmCancelToken));
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

  @override
  Future<RequestIdentity> customerCancel(String id, String cancelToken) async {
    calls.add('customerCancel');
    lastCancelledId = id;
    lastCancelledToken = cancelToken;
    final err = failCancel;
    if (err != null) {
      failCancel = null;
      throw KioskFailure(err);
    }
    serverStatus = 'CANCELLED';
    return RequestIdentity.fromJson(_identity());
  }
}

KioskL10n _testL10n() => KioskL10n.fromJson({
  'langs': [
    {'code': 'en', 'lang': 'en', 'name': 'English'},
    {'code': 'th', 'lang': 'th', 'name': 'ไทย'},
  ],
  'text': {
    'en': {
      'sentTitle': 'Your request has been sent',
      'orderNo': 'Order no.',
      'transactionNo': 'Transaction no.',
      'notCreated': 'Not created yet',
      'status': 'Status',
      'stWAITING': 'Waiting for staff',
      'stACCEPTED': 'Checking ID',
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
      'requestCancelled':
          'Your request has been cancelled. Please contact our staff.',
      'errCustomerCancel':
          'Unable to cancel at the kiosk. Please contact our staff.',
      'errGeneric': 'generic-en',
    },
    'th': {
      'cancelQueue': 'ยกเลิกคิว',
      'requestCancelled': 'คำขอของคุณถูกยกเลิก กรุณาติดต่อพนักงาน',
      'errCustomerCancel': 'ยกเลิกคิวที่ตู้ไม่ได้ กรุณาติดต่อพนักงาน',
    },
  },
  'errors': {'ยกเลิกคิวที่ตู้ไม่ได้ กรุณาติดต่อพนักงาน': 'errCustomerCancel'},
});

void main() {
  late _CancelFakeRepository server;

  setUp(() => server = _CancelFakeRepository());

  Future<ProviderContainer> reachWaitingInWidget(WidgetTester tester) async {
    final container = ProviderContainer(
      overrides: [
        kioskRepositoryProvider.overrideWithValue(server),
        kioskL10nProvider.overrideWithValue(_testL10n()),
        deviceLanguagesProvider.overrideWithValue(const ['en']),
      ],
    );
    container.listen(kioskControllerProvider, (_, _) {});
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

  testWidgets(
    'token kept and button shown: keeps confirm token across polls, hides token text, and cancels when tapped',
    (tester) async {
      final container = await reachWaitingInWidget(tester);

      expect(find.byKey(const Key('cancel-queue')), findsOneWidget);
      expect(find.text('Cancel queue'), findsOneWidget);
      expect(find.textContaining('tok-secret-99'), findsNothing);

      // Even after a status poll that carries no cancelToken, the token stays in memory and button remains shown.
      await tester.pump(const Duration(seconds: 5));
      expect(find.byKey(const Key('cancel-queue')), findsOneWidget);

      await tester.ensureVisible(find.byKey(const Key('cancel-queue')));
      await tester.tap(find.byKey(const Key('cancel-queue')));
      await tester.pump();

      expect(server.lastCancelledId, _CancelFakeRepository.id);
      expect(server.lastCancelledToken, 'tok-secret-99');
      expect(
        container.read(kioskControllerProvider).status!.status,
        KioskStatus.cancelled,
      );
      expect(find.byKey(const Key('cancel-queue')), findsNothing);
      expect(
        find.text('Your request has been cancelled. Please contact our staff.'),
        findsOneWidget,
      );

      await tester.pumpWidget(const SizedBox.shrink());
      container.dispose();
    },
  );

  testWidgets(
    'no token means no button: when confirm returns no cancelToken, WaitingPage hides the cancel button',
    (tester) async {
      server.confirmCancelToken = null;
      final container = await reachWaitingInWidget(tester);

      expect(container.read(kioskControllerProvider).canCancel, isFalse);
      expect(find.byKey(const Key('cancel-queue')), findsNothing);
      await tester.pumpWidget(const SizedBox.shrink());
      container.dispose();
    },
  );

  // #4243 ("มันควร กดยกเลิกได้ เลย" · "ให้ตั้งค่าได้เอง"):
  // the cancelled screen holds for kioskResultSecondsProvider (N = 30 s).
  test(
    'CANCELLED from a poll ends the flow: stops polling, shows cancelled error, and restarts after kioskResultSecondsProvider',
    () {
      fakeAsync((async) {
        final container = ProviderContainer(
          overrides: [
            kioskRepositoryProvider.overrideWithValue(server),
            kioskL10nProvider.overrideWithValue(_testL10n()),
            deviceLanguagesProvider.overrideWithValue(const ['th-TH']),
          ],
        );
        container.listen(kioskControllerProvider, (_, _) {});
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

        expect(container.read(kioskControllerProvider).canCancel, isTrue);

        // Next status poll sees CANCELLED (e.g. staff cancelled the request).
        server.serverStatus = 'CANCELLED';
        async.elapse(const Duration(seconds: 2));
        async.flushMicrotasks();

        final afterCancel = container.read(kioskControllerProvider);
        expect(afterCancel.status!.status, KioskStatus.cancelled);
        expect(afterCancel.canCancel, isFalse);
        expect(afterCancel.error, 'คำขอของคุณถูกยกเลิก กรุณาติดต่อพนักงาน');

        // Polling about the request has stopped.
        final callsAtCancel = List<String>.from(
          server.calls.where((call) => call != 'rates'),
        );
        final n = container.read(kioskResultSecondsProvider);
        async.elapse(Duration(seconds: n - 1));
        expect(
          server.calls.where((call) => call != 'rates').toList(),
          callsAtCancel,
        );
        expect(
          container.read(kioskControllerProvider).screen,
          KioskScreen.waiting,
        );

        // At N s after CANCELLED was seen, the kiosk resets to the select screen.
        async.elapse(const Duration(seconds: 1));
        async.flushMicrotasks();
        expect(
          container.read(kioskControllerProvider).screen,
          KioskScreen.select,
        );
        expect(container.read(kioskControllerProvider).quote, isNull);
        container.dispose();
      });
    },
  );
}
