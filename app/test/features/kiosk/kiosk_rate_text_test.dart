import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/features/kiosk/app/kiosk_providers.dart';
import 'package:xchange_kiosk/features/kiosk/domain/entities/kiosk_models.dart';
import 'package:xchange_kiosk/features/kiosk/domain/repository/kiosk_repository.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/controllers/kiosk_controller.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/pages/kiosk_page.dart';

import 'kiosk_controller_test.dart' show l10n;

class _RateServer implements KioskRepository {
  _RateServer({required this.code, required this.rate});

  final String code;
  final double rate;

  static const _id = '05eb0fcb-d41f-465e-84b6-83a5d26c3ae6';

  Map<String, dynamic> get _identity => {
    'id': _id,
    'queue': 'K-05EB0FCB',
    'orderNo': 'ORD-05EB0FCB',
    'transactionNo': null,
    'status': 'QUOTED',
  };

  @override
  Future<RatesModel> rates() async => RatesModel.fromJson({
    'branchId': 'MAIN',
    'branchName': 'XC-B',
    'open': true,
    'rates': [
      {'code': code, 'nameEn': code, 'buy': rate, 'sell': rate},
    ],
  });

  @override
  Future<QuoteModel> quote({
    required ExchangeType type,
    required String currency,
    required String amount,
    required String paymentMethod,
  }) async => QuoteModel.fromJson({
    ..._identity,
    'paymentMethod': paymentMethod,
    'type': type.wire,
    'currency': currency,
    'rate': rate,
    'foreign': 1000,
    'thb': 100,
  });

  @override
  Future<RequestIdentity> confirm(String id) async =>
      RequestIdentity.fromJson(_identity);

  @override
  Future<RequestIdentity> status(String id) async =>
      RequestIdentity.fromJson(_identity);

  @override
  Future<ReceiptModel> customerReceipt(String id) async =>
      ReceiptModel.fromJson({..._identity, 'image': null});

  @override
  Future<RequestIdentity> customerCancel(String id, String cancelToken) =>
      throw UnimplementedError();
}

void main() {
  late ProviderContainer container;

  Future<KioskController> pumpWithRate(
    WidgetTester tester, {
    required String code,
    required double rate,
  }) async {
    tester.view.physicalSize = const Size(1280, 1400);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);
    final server = _RateServer(code: code, rate: rate);
    container = ProviderContainer(
      overrides: [
        kioskRepositoryProvider.overrideWithValue(server),
        kioskL10nProvider.overrideWithValue(l10n()),
        deviceLanguagesProvider.overrideWithValue(const ['en']),
      ],
    );
    await tester.pumpWidget(
      UncontrolledProviderScope(
        container: container,
        child: const MaterialApp(home: KioskPage()),
      ),
    );
    final c = container.read(kioskControllerProvider.notifier);
    await c.start();
    await tester.pump();
    return c;
  }

  Future<void> finish(WidgetTester tester) async {
    await tester.pumpWidget(const SizedBox());
    container.dispose();
  }

  testWidgets('row 1: 32.1 (USD) is shown as 32.1000 on select and quote', (
    tester,
  ) async {
    final c = await pumpWithRate(tester, code: 'USD', rate: 32.1);
    expect(find.textContaining('= 32.1000 ฿ THB'), findsOneWidget);

    c
      ..setCurrency('USD')
      ..setAmount('100')
      ..setPaymentMethod('Cash');
    await c.submitQuote();
    await tester.pump();

    expect(find.text('1 USD = 32.1000 THB'), findsOneWidget);
    await finish(tester);
  });

  testWidgets(
    'row 2: 0.2105 (JPY 21.05/100) is shown as 0.2105 on select and quote',
    (tester) async {
      final c = await pumpWithRate(tester, code: 'JPY', rate: 0.2105);
      expect(find.textContaining('= 0.2105 ฿ THB'), findsOneWidget);

      c
        ..setCurrency('JPY')
        ..setAmount('1000')
        ..setPaymentMethod('Cash');
      await c.submitQuote();
      await tester.pump();

      expect(find.text('1 JPY = 0.2105 THB'), findsOneWidget);
      await finish(tester);
    },
  );

  testWidgets(
    'row 3: 0.02455 (KRW 24.55/1000) is shown as 0.02455 (not 0.0245) on select and quote',
    (tester) async {
      final c = await pumpWithRate(tester, code: 'KRW', rate: 0.02455);
      expect(find.textContaining('= 0.02455 ฿ THB'), findsOneWidget);
      expect(find.textContaining('= 0.0245 ฿ THB'), findsNothing);

      c
        ..setCurrency('KRW')
        ..setAmount('10000')
        ..setPaymentMethod('Cash');
      await c.submitQuote();
      await tester.pump();

      expect(find.text('1 KRW = 0.02455 THB'), findsOneWidget);
      expect(find.text('1 KRW = 0.0245 THB'), findsNothing);
      await finish(tester);
    },
  );

  testWidgets(
    'row 4: 0.0000245 is shown as 0.0000245 (not 0.0000) on select and quote',
    (tester) async {
      final c = await pumpWithRate(tester, code: 'IRR', rate: 0.0000245);
      expect(find.textContaining('= 0.0000245 ฿ THB'), findsOneWidget);
      expect(find.textContaining('= 0.0000 ฿ THB'), findsNothing);

      c
        ..setCurrency('IRR')
        ..setAmount('100000')
        ..setPaymentMethod('Cash');
      await c.submitQuote();
      await tester.pump();

      expect(find.text('1 IRR = 0.0000245 THB'), findsOneWidget);
      expect(find.text('1 IRR = 0.0000 THB'), findsNothing);
      await finish(tester);
    },
  );
}
