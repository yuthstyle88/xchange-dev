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
  String? failNext;

  static const id = '05eb0fcb-d41f-465e-84b6-83a5d26c3ae6';

  Map<String, dynamic> get _identity => {
    'id': id,
    'queue': 'K-05EB0FCB',
    'orderNo': 'ORD-05EB0FCB',
    'transactionNo': null,
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
      'currency': currency,
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

KioskL10n _l10n() => KioskL10n.fromJson({
  'langs': [
    {'code': 'en', 'lang': 'en', 'name': 'English'},
    {'code': 'th', 'lang': 'th', 'name': 'ไทย'},
    {'code': 'zh', 'lang': 'zh', 'name': '中文'},
  ],
  'text': {
    'en': {
      'closed': 'closed-en',
      'errGeneric': 'generic-en',
      'errSmall': 'small-en',
      'errReview': 'review-en',
      'quoteExpired': 'expired-en',
      'statusError': 'status-error-en',
    },
    'th': {
      'closed': 'closed-th',
      // Intentionally omit errGeneric in th so fallback to en is tested.
      'errSmall': 'small-th',
      'errReview': 'review-th',
      'quoteExpired': 'expired-th',
      'statusError': 'status-error-th',
    },
    'zh': {
      'closed': 'closed-zh',
      'errGeneric': 'generic-zh',
      'errSmall': 'small-zh',
      'errReview': 'review-zh',
      'quoteExpired': 'expired-zh',
      'statusError': 'status-error-zh',
    },
  },
  'errors': {
    'Amount is too small.': 'errSmall',
    'Please review your quote for 3 seconds.': 'errReview',
    'Untranslated literal sentence': 'Untranslated literal sentence',
  },
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
        kioskL10nProvider.overrideWithValue(_l10n()),
        deviceLanguagesProvider.overrideWithValue(languages),
      ],
    );
    container.listen(kioskControllerProvider, (_, _) {});
    return container.read(kioskControllerProvider.notifier);
  }

  void startAt0(FakeAsync async, {List<String> languages = const ['en']}) {
    c = make(languages: languages);
    c.start();
    async.flushMicrotasks();
  }

  void fillUsd() {
    c
      ..setCurrency('USD')
      ..setAmount('100')
      ..setPaymentMethod('Cash');
  }

  void quoteUsd(FakeAsync async, {List<String> languages = const ['en']}) {
    startAt0(async, languages: languages);
    fillUsd();
    c.submitQuote();
    async.flushMicrotasks();
  }

  setUp(() => server = FakeRepository());

  test('row 1: Quote refused with "Amount is too small." in en, then '
      'setLanguage(\'th\') shows the th text of errSmall', () {
    fakeAsync((async) {
      startAt0(async);
      fillUsd();
      server.failNext = 'Amount is too small.';
      c.submitQuote();
      async.flushMicrotasks();
      expect(st().error, 'small-en');

      c.setLanguage('th');
      expect(st().languageCode, 'th');
      expect(st().error, 'small-th');
      container.dispose();
    });
  });

  test('row 2: reportError(\'errGeneric\'), then switch language shows that '
      'language\'s errGeneric (and falls back to English when missing)', () {
    fakeAsync((async) {
      startAt0(async);
      c.reportError('errGeneric');
      expect(st().error, 'generic-en');

      c.setLanguage('th');
      expect(st().languageCode, 'th');
      expect(st().error, 'generic-en', reason: 'th falls back to en');

      c.setLanguage('zh');
      expect(st().languageCode, 'zh');
      expect(st().error, 'generic-zh');
      container.dispose();
    });
  });

  test('row 3: statusError showing, then switch language shows that '
      'language\'s statusError', () {
    fakeAsync((async) {
      quoteUsd(async);
      server.failNext = 'network down';
      async.elapse(const Duration(seconds: 5));
      expect(st().error, 'status-error-en');

      c.setLanguage('th');
      expect(st().languageCode, 'th');
      expect(st().error, 'status-error-th');

      // Subsequent successful status poll still clears the translated statusError.
      async.elapse(const Duration(seconds: 5));
      expect(st().error, '');
      container.dispose();
    });
  });

  test('row 4: No error, then switch keeps empty error, whereas an active '
      'error translates on switch and clears to empty once resolved', () {
    fakeAsync((async) {
      startAt0(async);
      expect(st().error, '');

      c.setLanguage('th');
      expect(st().error, '');

      fillUsd();
      server.failNext = 'Amount is too small.';
      c.submitQuote();
      async.flushMicrotasks();
      expect(st().error, 'small-th');

      c.setLanguage('en');
      expect(st().error, 'small-en');

      c.submitQuote();
      async.flushMicrotasks();
      expect(st().screen, KioskScreen.quote);
      expect(st().error, '');

      c.setLanguage('th');
      expect(st().error, '');
      container.dispose();
    });
  });

  test('row 5: Closed counter on select, then switch shows that language\'s '
      'closed, and after opening a refusal also follows language switches', () {
    fakeAsync((async) {
      server.open = false;
      startAt0(async);
      expect(st().error, 'closed-en');

      c.setLanguage('th');
      expect(st().error, 'closed-th');

      server.open = true;
      async.elapse(const Duration(seconds: 30));
      async.flushMicrotasks();
      expect(st().error, '');

      fillUsd();
      server.failNext = 'Amount is too small.';
      c.submitQuote();
      async.flushMicrotasks();
      expect(st().error, 'small-th');

      c.setLanguage('en');
      expect(st().error, 'small-en');
      container.dispose();
    });
  });

  test('row 6: Expired quote, then switch shows that language\'s quoteExpired, '
      'and a confirm refusal before expiry also follows language switches', () {
    fakeAsync((async) {
      quoteUsd(async);
      async.elapse(const Duration(seconds: 3));
      server.failNext = 'Please review your quote for 3 seconds.';
      c.confirm();
      async.flushMicrotasks();
      expect(st().error, 'review-en');

      c.setLanguage('th');
      expect(st().error, 'review-th');

      server.serverStatus = 'EXPIRED';
      async.elapse(const Duration(seconds: 2));
      expect(st().quoteExpired, isTrue);
      expect(st().error, 'expired-th');

      c.setLanguage('en');
      expect(st().error, 'expired-en');
      container.dispose();
    });
  });

  test('row 7: Refusal shown in the kiosk language and re-translated when the '
      'customer switches language', () {
    fakeAsync((async) {
      quoteUsd(async, languages: const ['th-TH']);
      async.elapse(const Duration(seconds: 3));
      server.failNext = 'Please review your quote for 3 seconds.';
      c.confirm();
      async.flushMicrotasks();
      expect(st().error, 'review-th');

      c.setLanguage('en');
      expect(st().error, 'review-en');

      c.setLanguage('zh');
      expect(st().error, 'review-zh');
      container.dispose();
    });
  });

  test('row 8: Idle reset returns to the device language and clears a refusal '
      'that had followed a language switch', () {
    fakeAsync((async) {
      startAt0(async, languages: const ['en']);
      fillUsd();
      server.failNext = 'Amount is too small.';
      c.activity();
      c.submitQuote();
      async.flushMicrotasks();
      expect(st().error, 'small-en');

      c.activity();
      c.setLanguage('th');
      expect(st().languageCode, 'th');
      expect(st().error, 'small-th');

      async.elapse(const Duration(minutes: 2));
      async.flushMicrotasks();
      expect(st().languageCode, 'en');
      expect(st().error, '');
      container.dispose();
    });
  });

  test('edge case: A server sentence with no text key is re-shown as is '
      '(or generic fallback) across language switches without being wiped', () {
    fakeAsync((async) {
      startAt0(async);
      fillUsd();
      server.failNext = 'Untranslated literal sentence';
      c.submitQuote();
      async.flushMicrotasks();
      expect(st().error, 'Untranslated literal sentence');

      c.setLanguage('th');
      expect(st().error, 'Untranslated literal sentence');

      c.reportError('Raw server sentence with no key');
      expect(st().error, 'Raw server sentence with no key');

      c.setLanguage('en');
      expect(st().error, 'Raw server sentence with no key');
      container.dispose();
    });
  });
}
