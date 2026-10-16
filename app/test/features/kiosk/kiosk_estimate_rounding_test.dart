import 'package:fake_async/fake_async.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/features/kiosk/app/kiosk_providers.dart';
import 'package:xchange_kiosk/features/kiosk/domain/entities/kiosk_models.dart';
import 'package:xchange_kiosk/features/kiosk/domain/repository/kiosk_repository.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/controllers/kiosk_controller.dart';
import 'package:xchange_kiosk/l10n/kiosk_l10n.dart';

class _RatesRepo implements KioskRepository {
  _RatesRepo(this.ratesList);

  final List<Map<String, Object>> ratesList;

  @override
  Future<RatesModel> rates() async => RatesModel.fromJson({
    'branchId': 'MAIN',
    'branchName': 'XC-B',
    'open': true,
    'rates': ratesList,
  });

  @override
  Future<QuoteModel> quote({
    required ExchangeType type,
    required String currency,
    required String amount,
    required String paymentMethod,
  }) => throw UnimplementedError();

  @override
  Future<RequestIdentity> confirm(String id) => throw UnimplementedError();

  @override
  Future<RequestIdentity> status(String id) => throw UnimplementedError();

  @override
  Future<ReceiptModel> customerReceipt(String id) => throw UnimplementedError();
}

KioskL10n _l10n() => KioskL10n.fromJson({
  'langs': [
    {'code': 'en', 'lang': 'en', 'name': 'English'},
  ],
  'text': {'en': <String, String>{}},
  'errors': <String, String>{},
});

void main() {
  late ProviderContainer container;
  late KioskController c;
  KioskState st() => container.read(kioskControllerProvider);

  KioskController startWithRates(
    FakeAsync async,
    List<Map<String, Object>> rates,
  ) {
    container = ProviderContainer(
      overrides: [
        kioskRepositoryProvider.overrideWithValue(_RatesRepo(rates)),
        kioskL10nProvider.overrideWithValue(_l10n()),
        deviceLanguagesProvider.overrideWithValue(const ['en']),
      ],
    );
    container.listen(kioskControllerProvider, (_, _) {});
    c = container.read(kioskControllerProvider.notifier);
    c.start();
    async.flushMicrotasks();
    return c;
  }

  test('BUY 110 at per-1 rate 0.2105 (JPY 21.05/100) estimates 23.16 THB', () {
    fakeAsync((async) {
      startWithRates(async, [
        {
          'code': 'JPY',
          'nameEn': 'Japanese Yen',
          'buy': 0.2105,
          'sell': 0.2195,
        },
      ]);
      c
        ..setCurrency('JPY')
        ..setType(ExchangeType.buy)
        ..setAmount('110');
      expect(st().estimate!.foreign, 110.0);
      expect(st().estimate!.receive, 23.16);
      container.dispose();
    });
  });

  test('BUY 10 at 0.2115 estimates 2.12 THB', () {
    fakeAsync((async) {
      startWithRates(async, [
        {
          'code': 'JPY',
          'nameEn': 'Japanese Yen',
          'buy': 0.2115,
          'sell': 0.2195,
        },
      ]);
      c
        ..setCurrency('JPY')
        ..setType(ExchangeType.buy)
        ..setAmount('10');
      expect(st().estimate!.foreign, 10.0);
      expect(st().estimate!.receive, 2.12);
      container.dispose();
    });
  });

  test('BUY 100 at 32.1 estimates 3210.00 THB', () {
    fakeAsync((async) {
      startWithRates(async, [
        {'code': 'USD', 'nameEn': 'US Dollar', 'buy': 32.1, 'sell': 32.6},
        {
          'code': 'JPY',
          'nameEn': 'Japanese Yen',
          'buy': 0.2105,
          'sell': 0.2195,
        },
      ]);
      c
        ..setCurrency('USD')
        ..setType(ExchangeType.buy)
        ..setAmount('100');
      expect(st().estimate!.foreign, 100.0);
      expect(st().estimate!.receive, 3210.00);

      // Switching to JPY 110 on the same controller still rounds half-up (23.16).
      c
        ..setCurrency('JPY')
        ..setAmount('110');
      expect(st().estimate!.receive, 23.16);
      container.dispose();
    });
  });

  test('BUY 250.5 at 33 estimates 8266.50 THB', () {
    fakeAsync((async) {
      startWithRates(async, [
        {'code': 'USD', 'nameEn': 'US Dollar', 'buy': 33.0, 'sell': 33.5},
        {
          'code': 'JPY',
          'nameEn': 'Japanese Yen',
          'buy': 0.2115,
          'sell': 0.2195,
        },
      ]);
      c
        ..setCurrency('USD')
        ..setType(ExchangeType.buy)
        ..setAmount('250.5');
      expect(st().estimate!.foreign, 250.5);
      expect(st().estimate!.receive, 8266.50);

      c
        ..setCurrency('JPY')
        ..setAmount('10');
      expect(st().estimate!.receive, 2.12);
      container.dispose();
    });
  });

  test('SELL 1000 THB at 32.6 estimates 30.67 and cuts exact decimals toward '
      'zero', () {
    fakeAsync((async) {
      startWithRates(async, [
        {'code': 'USD', 'nameEn': 'US Dollar', 'buy': 32.1, 'sell': 32.6},
        {'code': 'KWD', 'nameEn': 'Kuwaiti Dinar', 'buy': 99.0, 'sell': 100.0},
      ]);
      c
        ..setCurrency('USD')
        ..setType(ExchangeType.sell)
        ..setAmount('1000');
      expect(st().estimate!.foreign, 30.67);
      expect(st().estimate!.receive, 30.67);

      // 100 / 32.6 = 3.067484... cuts toward zero to 3.06 (half-up would give 3.07).
      c.setAmount('100');
      expect(st().estimate!.foreign, 3.06);
      expect(st().estimate!.receive, 3.06);

      // 29 / 100 = 0.29 exactly (binary double 29 / 100 * 100 = 28.999999999999996 -> 0.28).
      c
        ..setCurrency('KWD')
        ..setAmount('29');
      expect(st().estimate!.foreign, 0.29);
      expect(st().estimate!.receive, 0.29);
      container.dispose();
    });
  });

  test(
    'amount abc, 0, -1, above 10,000,000, or rate 0 returns null estimate',
    () {
      fakeAsync((async) {
        startWithRates(async, [
          {'code': 'USD', 'nameEn': 'US Dollar', 'buy': 0.2105, 'sell': 0.0},
        ]);
        c
          ..setCurrency('USD')
          ..setType(ExchangeType.buy);
        for (final invalid in ['abc', '0', '-1', '10000000.01', '10000001']) {
          c.setAmount(invalid);
          expect(st().estimate, isNull, reason: 'amount "$invalid"');
        }

        // Valid amount 110 at rate 0.2105 gives 23.16 on BUY, and null on SELL (rate 0).
        c.setAmount('110');
        expect(st().estimate!.receive, 23.16);
        c.setType(ExchangeType.sell);
        expect(st().estimate, isNull, reason: 'sell rate is 0');
        container.dispose();
      });
    },
  );
}
