import 'package:fake_async/fake_async.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/features/kiosk/app/kiosk_providers.dart';
import 'package:xchange_kiosk/features/kiosk/domain/entities/kiosk_models.dart';
import 'package:xchange_kiosk/features/kiosk/domain/kiosk_failure.dart';
import 'package:xchange_kiosk/features/kiosk/domain/repository/kiosk_repository.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/controllers/kiosk_controller.dart';
import 'package:xchange_kiosk/l10n/kiosk_l10n.dart';

/// A stand-in for the counter's server that records calls and answers from
/// [serverStatus] / [receiptImage].
class FakeRepository implements KioskRepository {
  final calls = <String>[];
  Map<String, Object?>? lastQuote;
  bool open = true;
  String serverStatus = 'QUOTED';
  String? receiptImage;
  String? transactionNo;
  String? failNext;

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
    lastQuote = {
      'type': type.wire,
      'currency': currency,
      'amount': amount,
      'paymentMethod': paymentMethod,
    };
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
    {'code': 'th', 'lang': 'th', 'name': 'ไทย'},
    {'code': 'ar', 'lang': 'ar', 'name': 'العربية', 'dir': 'rtl'},
    {'code': 'zh-HK', 'lang': 'zh-Hant', 'name': '繁體中文'},
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
    'th': {'closed': 'closed-th', 'errReview': 'review-th'},
    'ar': <String, String>{},
    'zh-HK': <String, String>{},
  },
  'errors': {'Please review your quote for 3 seconds.': 'errReview'},
});

void main() {
  late FakeRepository server;
  late ProviderContainer container;
  late KioskController c;
  KioskState st() => container.read(kioskControllerProvider);

  KioskController make({List<String> languages = const ['en']}) {
    container = ProviderContainer(
      overrides: [
        kioskRepositoryProvider.overrideWithValue(server),
        kioskL10nProvider.overrideWithValue(l10n()),
        deviceLanguagesProvider.overrideWithValue(languages),
      ],
    );
    // Keep the controller alive the way the page's watch does.
    container.listen(kioskControllerProvider, (_, _) {});
    return container.read(kioskControllerProvider.notifier);
  }

  /// Brings the controller to the quote screen inside [async].
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

  /// From the quote screen: wait out the review and confirm.
  void confirmAfterReview(FakeAsync async) {
    async.elapse(const Duration(seconds: 3));
    c.confirm();
    async.flushMicrotasks();
  }

  setUp(() => server = FakeRepository());

  test('a closed counter loads rates but cannot be reviewed', () {
    fakeAsync((async) {
      server.open = false;
      c = make();
      c.start();
      async.flushMicrotasks();
      c
        ..setAmount('100')
        ..setPaymentMethod('Cash');
      expect(st().error, 'closed-en');
      expect(st().canReview, isFalse);
      container.dispose();
    });
  });

  test('review needs a valid amount and a payment method', () {
    fakeAsync((async) {
      c = make();
      c.start();
      async.flushMicrotasks();
      expect(st().currency, 'USD');
      c.setAmount('100');
      expect(st().canReview, isFalse, reason: 'no payment method yet');
      c.setPaymentMethod('Cash');
      expect(st().canReview, isTrue);
      for (final bad in ['', '0', '-5', 'abc', '10000001']) {
        c.setAmount(bad);
        expect(st().canReview, isFalse, reason: 'amount "$bad"');
      }
      container.dispose();
    });
  });

  test('the estimate follows the v2 rounding: buy rounds, sell floors', () {
    fakeAsync((async) {
      c = make();
      c.start();
      async.flushMicrotasks();
      c.setAmount('100');
      expect(st().estimate!.foreign, 100);
      expect(st().estimate!.receive, 3210);
      c
        ..setType(ExchangeType.sell)
        ..setAmount('1000');
      // 1000 / 32.6 = 30.6748… → floored to 30.67
      expect(st().estimate!.foreign, 30.67);
      expect(st().estimate!.receive, 30.67);
      container.dispose();
    });
  });

  test('switching to buy drops a payment method buy does not allow', () {
    fakeAsync((async) {
      c = make();
      c.start();
      async.flushMicrotasks();
      c
        ..setType(ExchangeType.sell)
        ..setPaymentMethod('Alipay')
        ..setType(ExchangeType.buy);
      expect(st().paymentMethod, isNull);
      expect(st().availablePaymentMethods, ['Cash', 'Bank Transfer']);
      container.dispose();
    });
  });

  test('the amount is sent as typed and the quote comes from the server', () {
    fakeAsync((async) {
      c = make();
      quoteUsd(async);
      expect(server.lastQuote, {
        'type': 'BUY',
        'currency': 'USD',
        'amount': '100',
        'paymentMethod': 'Cash',
      });
      expect(st().screen, KioskScreen.quote);
      expect(st().quote!.thb, 3210);
      container.dispose();
    });
  });

  test('confirm unlocks only after the three-second review', () {
    fakeAsync((async) {
      c = make();
      quoteUsd(async);
      expect(st().secondsLeft, 3);
      expect(st().canConfirm, isFalse);
      c.confirm();
      async.flushMicrotasks();
      expect(server.calls, isNot(contains('confirm')));
      async.elapse(const Duration(seconds: 2));
      expect(st().canConfirm, isFalse);
      async.elapse(const Duration(seconds: 1));
      expect(st().canConfirm, isTrue);
      c.confirm();
      async.flushMicrotasks();
      expect(st().screen, KioskScreen.waiting);
      expect(st().status!.status, KioskStatus.waiting);
      container.dispose();
    });
  });

  test('a server refusal is shown in the kiosk language, not as sent', () {
    fakeAsync((async) {
      c = make(languages: ['th-TH']);
      quoteUsd(async);
      async.elapse(const Duration(seconds: 3));
      server.failNext = 'Please review your quote for 3 seconds.';
      c.confirm();
      async.flushMicrotasks();
      expect(st().error, 'review-th');
      expect(st().screen, KioskScreen.quote);
      server.failNext = 'something the client has never heard of';
      c.confirm();
      async.flushMicrotasks();
      expect(st().error, 'generic-en', reason: 'th has no errGeneric');
      container.dispose();
    });
  });

  test('an expired quote locks confirm', () {
    fakeAsync((async) {
      c = make();
      quoteUsd(async);
      server.serverStatus = 'EXPIRED';
      async.elapse(const Duration(seconds: 5));
      expect(st().quoteExpired, isTrue);
      expect(st().canConfirm, isFalse);
      expect(st().error, 'expired-en');
      container.dispose();
    });
  });

  test('going back forgets the quote and stops asking about it', () {
    fakeAsync((async) {
      c = make();
      quoteUsd(async);
      c.backToSelect();
      final before = server.calls.length;
      async.elapse(const Duration(seconds: 30));
      expect(server.calls.length, before);
      expect(st().quote, isNull);
      expect(st().screen, KioskScreen.select);
      container.dispose();
    });
  });

  test('the receipt screen appears when staff publish an image, and leaves '
      'when it is gone', () {
    fakeAsync((async) {
      c = make();
      quoteUsd(async);
      confirmAfterReview(async);
      async.elapse(const Duration(seconds: 3));
      expect(st().screen, KioskScreen.waiting);
      server
        ..serverStatus = 'ID_CHECKED'
        ..receiptImage = 'data:image/png;base64,AAAA';
      async.elapse(const Duration(seconds: 3));
      expect(st().screen, KioskScreen.receipt);
      expect(st().receiptImage, 'data:image/png;base64,AAAA');
      server.receiptImage = null;
      async.elapse(const Duration(seconds: 3));
      expect(st().screen, KioskScreen.waiting);
      container.dispose();
    });
  });

  test('the kiosk restarts 30 seconds after it sees the request completed', () {
    fakeAsync((async) {
      c = make();
      quoteUsd(async);
      confirmAfterReview(async);
      server
        ..serverStatus = 'COMPLETED'
        ..transactionNo = 'XC-20261006-000001';
      // The status poll has run every 5 s since the quote (t=0); the confirm
      // was at t=3, so the poll that first sees COMPLETED is 2 s away.
      async.elapse(const Duration(seconds: 2));
      expect(st().status!.status, KioskStatus.completed);
      async.elapse(const Duration(seconds: 29));
      expect(st().quote, isNotNull, reason: '29 s after completion was seen');
      async.elapse(const Duration(seconds: 1));
      async.flushMicrotasks();
      expect(st().quote, isNull);
      expect(st().screen, KioskScreen.select);
      container.dispose();
    });
  });

  test('two idle minutes reset the first two screens but never the counter '
      'screens', () {
    fakeAsync((async) {
      c = make();
      quoteUsd(async);
      c.setLanguage('th');
      async.elapse(const Duration(minutes: 2));
      async.flushMicrotasks();
      expect(st().screen, KioskScreen.select);
      expect(st().quote, isNull);
      expect(st().languageCode, 'en', reason: 'back to the device language');

      c
        ..setAmount('100')
        ..setPaymentMethod('Cash');
      c.submitQuote();
      async.flushMicrotasks();
      confirmAfterReview(async);
      async.elapse(const Duration(minutes: 5));
      expect(st().screen, KioskScreen.waiting);
      expect(st().quote, isNotNull);
      container.dispose();
    });
  });

  test('activity postpones the idle reset', () {
    fakeAsync((async) {
      c = make();
      quoteUsd(async);
      async.elapse(const Duration(seconds: 100));
      c.activity();
      async.elapse(const Duration(seconds: 100));
      expect(st().screen, KioskScreen.quote);
      container.dispose();
    });
  });

  group('language', () {
    test('detection follows the device list', () {
      final l = l10n();
      expect(l.detect(['th-TH', 'en']), 'th');
      expect(l.detect(['zh-TW']), 'zh-HK');
      expect(l.detect(['zh-Hant-HK']), 'zh-HK');
      expect(l.detect(['xx', 'ar-EG']), 'ar');
      expect(l.detect(['xx']), 'en');
    });

    test('missing keys fall back to English, then to the key', () {
      final t = l10n().of('th');
      expect(t.text('closed'), 'closed-th');
      expect(t.text('errGeneric'), 'generic-en');
      expect(t.text('nope'), 'nope');
      expect(t.text('wait', {'n': 2}), 'Please review (2)');
    });

    test('an unknown language code gets English', () {
      expect(l10n().of('klingon').code, 'en');
    });
  });
}
