// Provided with ticket F8 (design docs/design/xchange-v3.md §5.5.2, §10 row F8),
// written before the change. Do not edit, delete or weaken these tests; add
// your own in another file.
//
// The kiosk offers only the channels the branch has switched on for the chosen
// direction, and shows the chosen channel's instruction, from the `rates`
// answer's `paymentMethods` — as v2's `public/kiosk.js:42`, `:51-54` of the
// 10-06 bundle do:
//
//   - a channel is offered when its row is `enabled` and on for the direction
//     (`buy` for BUY, `sell` for SELL); a channel with no row is not offered;
//   - a chosen channel that stops being offered (another direction, or a rates
//     read where it was switched off) is cleared, so review waits for a new one;
//   - the instruction of the chosen channel is shown ('' when it has none).
//
// [proposed in the design] When the answer has no `paymentMethods` (the
// deployed v2 server, or v3 before row K-HTTP) today's fixed lists stay.
import 'package:fake_async/fake_async.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/features/kiosk/app/kiosk_providers.dart';
import 'package:xchange_kiosk/features/kiosk/domain/entities/kiosk_models.dart';
import 'package:xchange_kiosk/features/kiosk/domain/repository/kiosk_repository.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/controllers/kiosk_controller.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/pages/select_page.dart';
import 'package:xchange_kiosk/l10n/kiosk_l10n.dart';

const _transferNote = 'If transfer, call agent · หากโอนเงิน กรุณาติดต่อพนักงาน';

/// v2's defaults (`payment-settings.mjs:3`), as `rates` prints them: no
/// `account` (`publicMethods`, `:18`).
List<Map<String, Object>> defaults() => [
  {'id': 'Cash', 'enabled': true, 'buy': true, 'sell': true, 'instruction': ''},
  {
    'id': 'Credit Card',
    'enabled': true,
    'buy': false,
    'sell': true,
    'instruction': '',
  },
  {
    'id': 'Alipay',
    'enabled': true,
    'buy': false,
    'sell': true,
    'instruction': '',
  },
  {
    'id': 'PromptPay',
    'enabled': true,
    'buy': false,
    'sell': true,
    'instruction': '',
  },
  {
    'id': 'Bank Transfer',
    'enabled': true,
    'buy': true,
    'sell': true,
    'instruction': _transferNote,
  },
];

List<Map<String, Object>> changed(String id, Map<String, Object> change) => [
  for (final row in defaults()) row['id'] == id ? {...row, ...change} : row,
];

/// The counter's server: `rates` answers [methods] as `paymentMethods`, or
/// leaves the key out when it is null.
class PaymentServer implements KioskRepository {
  List<Map<String, Object>>? methods = defaults();
  int ratesReads = 0;

  @override
  Future<RatesModel> rates() async {
    ratesReads++;
    return RatesModel.fromJson({
      'branchId': 'MAIN',
      'branchName': 'XC-B',
      'open': true,
      'rates': [
        {'code': 'USD', 'nameEn': 'US Dollar', 'buy': 32.1, 'sell': 32.6},
      ],
      if (methods != null) 'paymentMethods': methods,
      'simulation': false,
    });
  }

  @override
  Future<QuoteModel> quote({
    required ExchangeType type,
    required String currency,
    required String amount,
    required String paymentMethod,
  }) async => throw UnimplementedError('not asked here');

  @override
  Future<RequestIdentity> confirm(String id) async =>
      throw UnimplementedError('not asked here');

  @override
  Future<RequestIdentity> status(String id) async =>
      throw UnimplementedError('not asked here');

  @override
  Future<ReceiptModel> customerReceipt(String id) async =>
      throw UnimplementedError('not asked here');

  @override
  Future<RequestIdentity> customerCancel(String id, String cancelToken) async =>
      throw UnimplementedError('not asked here');
}

KioskL10n texts() => KioskL10n.fromJson({
  'langs': [
    {'code': 'en', 'lang': 'en', 'name': 'English'},
  ],
  'text': {
    'en': {
      'payment': 'Payment',
      'choosePayment': 'Choose a payment method',
      'cash': 'Cash',
      'creditCard': 'Credit Card',
      'transfer': 'Bank Transfer',
    },
  },
  'errors': <String, String>{},
});

void main() {
  late PaymentServer server;
  late ProviderContainer container;
  late KioskController c;
  KioskState st() => container.read(kioskControllerProvider);

  KioskController make() {
    container = ProviderContainer(
      overrides: [
        kioskRepositoryProvider.overrideWithValue(server),
        kioskL10nProvider.overrideWithValue(texts()),
        deviceLanguagesProvider.overrideWithValue(const ['en']),
      ],
    );
    container.listen(kioskControllerProvider, (_, _) {});
    return container.read(kioskControllerProvider.notifier);
  }

  /// Started, with USD and an amount, on [type].
  void ready(FakeAsync async, ExchangeType type) {
    c = make();
    c.start();
    async.flushMicrotasks();
    c
      ..setType(type)
      ..setCurrency('USD')
      ..setAmount('100');
  }

  setUp(() => server = PaymentServer());

  test('with the defaults the lists are v2\'s: SELL every channel, BUY cash '
      'and transfer', () {
    fakeAsync((async) {
      ready(async, ExchangeType.sell);
      expect(st().availablePaymentMethods, [
        'Credit Card',
        'Alipay',
        'PromptPay',
        'Cash',
        'Bank Transfer',
      ]);
      c.setType(ExchangeType.buy);
      expect(st().availablePaymentMethods, ['Cash', 'Bank Transfer']);
      container.dispose();
    });
  });

  test('a channel switched off is not offered in either direction', () {
    fakeAsync((async) {
      server.methods = changed('Credit Card', {'enabled': false});
      ready(async, ExchangeType.sell);
      expect(st().availablePaymentMethods, [
        'Alipay',
        'PromptPay',
        'Cash',
        'Bank Transfer',
      ]);
      container.dispose();
    });
  });

  test('a channel is offered only for the direction it is on for', () {
    fakeAsync((async) {
      server.methods = changed('Cash', {'buy': false});
      ready(async, ExchangeType.buy);
      expect(st().availablePaymentMethods, ['Bank Transfer']);
      c.setType(ExchangeType.sell);
      expect(st().availablePaymentMethods, contains('Cash'));

      server.methods = changed('Bank Transfer', {'sell': false});
      c.reset();
      async.flushMicrotasks();
      c.setType(ExchangeType.sell);
      expect(st().availablePaymentMethods, [
        'Credit Card',
        'Alipay',
        'PromptPay',
        'Cash',
      ]);
      c.setType(ExchangeType.buy);
      expect(st().availablePaymentMethods, ['Cash', 'Bank Transfer']);
      container.dispose();
    });
  });

  test('a channel the answer does not list is not offered', () {
    fakeAsync((async) {
      server.methods = [
        for (final row in defaults())
          if (row['id'] != 'Alipay') row,
      ];
      ready(async, ExchangeType.sell);
      expect(st().availablePaymentMethods, isNot(contains('Alipay')));
      expect(st().availablePaymentMethods, hasLength(4));
      container.dispose();
    });
  });

  test('an answer without paymentMethods keeps today\'s fixed lists', () {
    fakeAsync((async) {
      server.methods = null;
      ready(async, ExchangeType.sell);
      expect(st().availablePaymentMethods, paymentMethods);
      c.setType(ExchangeType.buy);
      expect(st().availablePaymentMethods, buyPaymentMethods);
      c.setPaymentMethod('Cash');
      expect(st().canReview, isTrue);
      container.dispose();
    });
  });

  test('a chosen channel that is not offered for the new direction is '
      'cleared', () {
    fakeAsync((async) {
      server.methods = changed('Cash', {'buy': false});
      ready(async, ExchangeType.sell);
      c.setPaymentMethod('Cash');
      expect(st().canReview, isTrue);
      c.setType(ExchangeType.buy);
      expect(st().paymentMethod, isNull);
      expect(st().canReview, isFalse);
      container.dispose();
    });
  });

  test('a channel switched off while chosen is cleared at the next rates '
      'read', () {
    fakeAsync((async) {
      ready(async, ExchangeType.sell);
      c.setPaymentMethod('Alipay');
      expect(st().canReview, isTrue);
      server.methods = changed('Alipay', {'enabled': false});
      final reads = server.ratesReads;
      async.elapse(KioskController.ratesRefresh);
      async.flushMicrotasks();
      expect(server.ratesReads, greaterThan(reads));
      expect(st().availablePaymentMethods, isNot(contains('Alipay')));
      expect(st().paymentMethod, isNull);
      expect(st().canReview, isFalse);
      container.dispose();
    });
  });

  test('review waits while the chosen channel is off for the direction', () {
    fakeAsync((async) {
      server.methods = changed('PromptPay', {'enabled': false});
      ready(async, ExchangeType.sell);
      c.setPaymentMethod('PromptPay');
      expect(st().canReview, isFalse);
      c.setPaymentMethod('Cash');
      expect(st().canReview, isTrue);
      container.dispose();
    });
  });

  Future<void> showSelectPage(WidgetTester tester) async {
    tester.view.physicalSize = const Size(1280, 3000);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    await tester.pumpWidget(
      UncontrolledProviderScope(
        container: container,
        child: const MaterialApp(
          home: Scaffold(body: SingleChildScrollView(child: SelectPage())),
        ),
      ),
    );
    await tester.pump();
  }

  Future<void> closePage(WidgetTester tester) async {
    await tester.pumpWidget(const SizedBox());
    container.dispose();
  }

  testWidgets('the chosen channel\'s instruction is shown, and nothing when it '
      'has none', (tester) async {
    server.methods = changed('PromptPay', {'instruction': 'Scan the code'});
    c = make();
    await c.start();
    c
      ..setType(ExchangeType.sell)
      ..setCurrency('USD')
      ..setAmount('100');
    await showSelectPage(tester);
    expect(find.text('Scan the code'), findsNothing, reason: 'nothing chosen');

    c.setPaymentMethod('PromptPay');
    await tester.pump();
    expect(find.text('Scan the code'), findsOneWidget);

    c.setPaymentMethod('Bank Transfer');
    await tester.pump();
    expect(find.text('Scan the code'), findsNothing);
    expect(find.text(_transferNote), findsOneWidget);

    c.setPaymentMethod('Cash');
    await tester.pump();
    expect(find.text(_transferNote), findsNothing);
    await closePage(tester);
  });

  testWidgets('the select page shows the chosen channel\'s instruction and '
      'does not offer a channel that is off', (tester) async {
    server.methods = [
      for (final row in changed('Credit Card', {'enabled': false}))
        row['id'] == 'Cash'
            ? {...row, 'instruction': 'Pay at the counter'}
            : row,
    ];
    c = make();
    await c.start();
    c
      ..setType(ExchangeType.sell)
      ..setCurrency('USD')
      ..setAmount('100')
      ..setPaymentMethod('Cash');
    await showSelectPage(tester);
    expect(find.text('Pay at the counter'), findsOneWidget);

    await tester.tap(find.byType(DropdownButtonFormField<String>));
    await tester.pumpAndSettle();
    expect(find.text('Credit Card'), findsNothing);
    expect(find.text('Alipay'), findsWidgets);
    await closePage(tester);
  });
}
