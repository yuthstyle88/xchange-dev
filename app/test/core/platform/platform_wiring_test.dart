// Provided with ticket A2. These tests are the specification: make them pass,
// do not edit, delete or weaken them.
//
// The kiosk reaches the device and the names only through two providers, so
// a test (and the other target) can put its own implementation there.
import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/core/platform/device_actions.dart';
import 'package:xchange_kiosk/core/platform/display_names.dart';
import 'package:xchange_kiosk/features/kiosk/app/kiosk_providers.dart';
import 'package:xchange_kiosk/features/kiosk/domain/entities/kiosk_models.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/controllers/kiosk_controller.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/pages/receipt_page.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/pages/select_page.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/pages/waiting_page.dart';
import 'package:xchange_kiosk/l10n/kiosk_l10n.dart';

const _png =
    'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlE'
    'QVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==';

/// Device actions that record what the pages ask and can be told to fail.
class _Device extends DeviceActions {
  _Device({this.languageList = const ['en'], this.fail = false});
  final List<String> languageList;
  final bool fail;
  final saved = <(String, String)>[];
  final opened = <String>[];

  @override
  List<String> languages() => languageList;

  @override
  Future<void> saveImage(String dataUrl, String filename) async {
    saved.add((dataUrl, filename));
    if (fail) {
      throw const DeviceActionException('the test says no');
    }
  }

  @override
  Future<void> openHttps(Uri url) async {
    opened.add('$url');
    if (fail) {
      throw const DeviceActionException('the test says no');
    }
  }
}

/// Names that show which arguments they were asked with.
class _Names implements DisplayNames {
  const _Names({this.silent = false});
  final bool silent;

  @override
  String currencySymbol(String code) => silent ? code : 'S!';

  @override
  String? currencyName(String lang, String code) =>
      silent ? null : 'N-$lang-$code';

  @override
  String? regionName(String lang, String region) =>
      silent ? null : 'R-$lang-$region';
}

/// The kiosk controller parked on one screen: no server, no timers.
class _Parked extends KioskController {
  _Parked(this._state);
  final KioskState _state;

  @override
  KioskState build() => _state;
}

KioskL10n realTexts() => KioskL10n.fromJson(
  jsonDecode(File('assets/l10n/kiosk.json').readAsStringSync())
      as Map<String, dynamic>,
);

Map<String, dynamic> _identity(String status, {String? transactionNo}) => {
  'id': '05eb0fcb-d41f-465e-84b6-83a5d26c3ae6',
  'queue': 'K-05EB0FCB',
  'orderNo': 'ORD-05EB0FCB',
  'transactionNo': transactionNo,
  'status': status,
};

QuoteModel _quote() => QuoteModel.fromJson({
  ..._identity('WAITING'),
  'paymentMethod': 'Cash',
  'type': ExchangeType.buy.wire,
  'currency': 'USD',
  'rate': 32.1,
  'foreign': 100,
  'thb': 3210,
});

RatesModel _rates() => RatesModel.fromJson({
  'branchId': 'MAIN',
  'branchName': 'XC-B',
  'open': true,
  'rates': [
    {
      'code': 'USD',
      'name': 'ดอลลาร์สหรัฐ',
      'nameEn': 'US Dollar',
      'buy': 32.1,
      'sell': 32.6,
      'countries': [
        {'code': 'US', 'en': 'United States', 'th': 'สหรัฐอเมริกา'},
      ],
    },
    {
      'code': 'JPY',
      'name': 'เยน',
      'nameEn': 'Japanese Yen',
      'buy': 0.211,
      'sell': 0.219,
      'countries': [
        {'code': 'JP', 'en': 'Japan', 'th': 'ญี่ปุ่น'},
      ],
    },
  ],
});

void main() {
  late ProviderContainer container;

  /// A container whose kiosk is parked on [state].
  ProviderContainer parked(
    KioskState state, {
    DeviceActions? device,
    DisplayNames? names,
  }) {
    container = ProviderContainer(
      overrides: [
        kioskL10nProvider.overrideWithValue(realTexts()),
        kioskControllerProvider.overrideWith(() => _Parked(state)),
        if (device != null) deviceActionsProvider.overrideWithValue(device),
        if (names != null) displayNamesProvider.overrideWithValue(names),
      ],
    );
    addTearDown(container.dispose);
    return container;
  }

  Future<void> show(WidgetTester tester, Widget page) async {
    tester.view.physicalSize = const Size(1280, 3000);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    await tester.pumpWidget(
      UncontrolledProviderScope(
        container: container,
        child: MaterialApp(
          home: Scaffold(body: SingleChildScrollView(child: page)),
        ),
      ),
    );
    await tester.pump();
  }

  String error() => container.read(kioskControllerProvider).error;

  group('providers', () {
    test('on a device the defaults are the device implementations', () {
      final c = ProviderContainer();
      addTearDown(c.dispose);
      expect(c.read(deviceActionsProvider), isA<NativeDeviceActions>());
      expect(createDeviceActions(), isA<NativeDeviceActions>());
      expect(c.read(displayNamesProvider), isA<BundledDisplayNames>());
    });

    test('the device languages come from the device actions, in their '
        'order', () {
      final c = ProviderContainer(
        overrides: [
          deviceActionsProvider.overrideWithValue(
            _Device(languageList: const ['ja-JP', 'th-TH', 'en']),
          ),
        ],
      );
      addTearDown(c.dispose);
      expect(c.read(deviceLanguagesProvider), ['ja-JP', 'th-TH', 'en']);
    });

    test('the kiosk starts in the first device language it has', () {
      for (final (languages, want) in [
        (const ['th-TH', 'ja-JP'], 'th'),
        (const ['ja-JP', 'th-TH'], 'ja'),
        (const ['zh-Hant-TW', 'en'], 'zh-HK'),
        (const ['sv-SE'], 'en'),
      ]) {
        final c = ProviderContainer(
          overrides: [
            kioskL10nProvider.overrideWithValue(realTexts()),
            deviceActionsProvider.overrideWithValue(
              _Device(languageList: languages),
            ),
          ],
        );
        addTearDown(c.dispose);
        expect(
          c.read(kioskControllerProvider).languageCode,
          want,
          reason: '$languages',
        );
      }
    });
  });

  group('KioskController.reportError', () {
    test('puts the sentence for a text key, in the language on screen, into '
        'the kiosk\'s one error line', () {
      final texts = realTexts();
      parked(const KioskState(languageCode: 'th'));
      container.read(kioskControllerProvider.notifier).reportError('closed');
      expect(error(), texts.of('th').text('closed'));
      expect(error(), isNot(texts.of('en').text('closed')));

      container
          .read(kioskControllerProvider.notifier)
          .reportError('errGeneric');
      expect(error(), texts.of('th').text('errGeneric'));
    });

    test('changes nothing else on the screen', () {
      final before = KioskState(
        languageCode: 'en',
        screen: KioskScreen.receipt,
        quote: _quote(),
        receiptImage: _png,
      );
      parked(before);
      container
          .read(kioskControllerProvider.notifier)
          .reportError('errGeneric');
      final after = container.read(kioskControllerProvider);
      expect(after.error, realTexts().of('en').text('errGeneric'));
      expect(after.screen, KioskScreen.receipt);
      expect(after.receiptImage, _png);
      expect(after.quote, same(before.quote));
      expect(after.languageCode, 'en');
    });
  });

  group('receipt page', () {
    KioskState receipt({String? transactionNo}) => KioskState(
      languageCode: 'en',
      screen: KioskScreen.receipt,
      quote: _quote(),
      status: RequestIdentity.fromJson(
        _identity('COMPLETED', transactionNo: transactionNo),
      ),
      receiptImage: _png,
    );

    testWidgets('"save" hands the image and its file name to the device '
        'actions', (tester) async {
      final device = _Device();
      parked(receipt(transactionNo: 'TX-77'), device: device);
      await show(tester, const ReceiptPage());

      await tester.tap(find.byKey(const Key('save-receipt')));
      await tester.pump();

      expect(device.saved, [(_png, 'TX-77_RECEIPT.png')]);
      expect(error(), isEmpty);
      expect(find.byType(ReceiptPage), findsOneWidget);
    });

    testWidgets('before the transaction has a number the file is named after '
        'the queue number, as on the web', (tester) async {
      final device = _Device();
      parked(receipt(), device: device);
      await show(tester, const ReceiptPage());

      await tester.tap(find.byKey(const Key('save-receipt')));
      await tester.pump();

      expect(device.saved.single.$2, 'K-05EB0FCB_PENDING_RECEIPT.png');
    });

    testWidgets('when saving fails the kiosk shows its generic error and the '
        'receipt stays', (tester) async {
      final device = _Device(fail: true);
      parked(receipt(transactionNo: 'TX-77'), device: device);
      await show(tester, const ReceiptPage());

      await tester.tap(find.byKey(const Key('save-receipt')));
      await tester.pump();

      expect(device.saved.length, 1);
      expect(error(), realTexts().of('en').text('errGeneric'));
      expect(container.read(kioskControllerProvider).receiptImage, _png);
      expect(find.byType(ReceiptPage), findsOneWidget);
      expect(tester.takeException(), isNull);
    });
  });

  group('waiting page', () {
    KioskState waiting() => KioskState(
      languageCode: 'en',
      screen: KioskScreen.waiting,
      quote: _quote(),
      status: RequestIdentity.fromJson(_identity('WAITING')),
    );

    testWidgets('the Telegram button opens the link through the device '
        'actions', (tester) async {
      final device = _Device();
      parked(waiting(), device: device);
      await show(tester, const WaitingPage());

      await tester.tap(find.text('@XCHANGEXTK'));
      await tester.pump();

      expect(device.opened, ['https://t.me/XCHANGEXTK']);
      expect(error(), isEmpty);
    });

    testWidgets('when nothing can open the link the kiosk shows its generic '
        'error and the page, with its QR code, stays', (tester) async {
      final device = _Device(fail: true);
      parked(waiting(), device: device);
      await show(tester, const WaitingPage());

      await tester.tap(find.text('@XCHANGEXTK'));
      await tester.pump();

      expect(device.opened.length, 1);
      expect(error(), realTexts().of('en').text('errGeneric'));
      expect(find.byType(WaitingPage), findsOneWidget);
      expect(
        find.bySemanticsLabel('QR code linking to https://t.me/XCHANGEXTK'),
        findsOneWidget,
      );
      expect(tester.takeException(), isNull);
    });
  });

  group('select page', () {
    KioskState select(String languageCode) => KioskState(
      languageCode: languageCode,
      model: _rates(),
      currency: 'USD',
    );

    testWidgets('currency symbols and names come from the display names, '
        'asked with the language TAG of the language on screen', (
      tester,
    ) async {
      // Traditional Chinese: kiosk code zh-HK, language tag zh-Hant.
      parked(select('zh-HK'), names: const _Names());
      await show(tester, const SelectPage());

      final usd = find.byKey(const Key('currency-USD'));
      expect(usd, findsOneWidget);
      expect(
        find.descendant(of: usd, matching: find.text('S! USD')),
        findsOneWidget,
      );
      expect(
        find.descendant(of: usd, matching: find.text('N-zh-Hant-USD')),
        findsOneWidget,
      );
      expect(
        find.descendant(
          of: find.byKey(const Key('currency-JPY')),
          matching: find.text('N-zh-Hant-JPY'),
        ),
        findsOneWidget,
      );
    });

    testWidgets('a currency can be found by the name of its region in the '
        'language on screen', (tester) async {
      parked(select('zh-HK'), names: const _Names());
      await show(tester, const SelectPage());

      await tester.enterText(find.byType(TextField).first, 'r-zh-hant-jp');
      await tester.pump();

      expect(find.byKey(const Key('currency-JPY')), findsOneWidget);
      expect(find.byKey(const Key('currency-USD')), findsNothing);
    });

    testWidgets('where the names know nothing the page shows the code as the '
        'symbol and the name the server sent', (tester) async {
      parked(select('en'), names: const _Names(silent: true));
      await show(tester, const SelectPage());

      final usd = find.byKey(const Key('currency-USD'));
      expect(
        find.descendant(of: usd, matching: find.text('USD USD')),
        findsOneWidget,
      );
      expect(
        find.descendant(of: usd, matching: find.text('US Dollar')),
        findsOneWidget,
      );
    });

    testWidgets('in Thai the server\'s Thai name is the fallback', (
      tester,
    ) async {
      parked(select('th'), names: const _Names(silent: true));
      await show(tester, const SelectPage());

      expect(
        find.descendant(
          of: find.byKey(const Key('currency-USD')),
          matching: find.text('ดอลลาร์สหรัฐ'),
        ),
        findsOneWidget,
      );
    });
  });
}
