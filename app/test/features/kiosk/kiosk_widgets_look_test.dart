import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/features/kiosk/domain/entities/kiosk_models.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/widgets/kiosk_widgets.dart';
import 'package:xchange_kiosk/l10n/kiosk_l10n.dart';
import 'package:xchange_kiosk/shared/theme/colors.dart';

KioskTexts _testL10n() {
  final l10n = KioskL10n.fromJson({
    'langs': [
      {'code': 'en', 'lang': 'en', 'name': 'English'},
    ],
    'text': {
      'en': {
        'youGive': 'You give',
        'payment': 'Payment',
        'rate': 'Rate',
        'youReceive': 'You receive',
        'creditCard': 'Credit Card',
        'cash': 'Cash',
        'transfer': 'Transfer',
        'orderNo': 'Order number',
        'transactionNo': 'Transaction number',
        'notCreated': 'Not created',
        'status': 'Status',
        'stepId': 'ID Checked',
        'stepPending': 'Processing',
        'stepSuccess': 'Success',
        'phasePassport': 'Please present passport',
        'phaseProcessing': 'Processing exchange',
        'phaseDone': 'Complete',
        'phaseWaiting': 'Please wait',
      },
    },
    'errors': <String, String>{},
  });
  return l10n.of('en');
}

void main() {
  testWidgets(
    'Button height, padding, corner: min height 68, padding 16, radius 14, on both buttons',
    (tester) async {
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: Column(
              children: [
                PrimaryButton(label: 'Primary', onPressed: () {}),
                SecondaryButton(label: 'Secondary', onPressed: () {}),
              ],
            ),
          ),
        ),
      );

      final primaryFilled = tester.widget<FilledButton>(
        find.byType(FilledButton),
      );
      final primaryStyle = primaryFilled.style!;
      expect(
        primaryStyle.minimumSize?.resolve({}),
        const Size(double.infinity, 68),
      );
      expect(primaryStyle.padding?.resolve({}), const EdgeInsets.all(16));
      final primaryShape =
          primaryStyle.shape?.resolve({}) as RoundedRectangleBorder;
      expect(primaryShape.borderRadius, BorderRadius.circular(14));

      final secondaryOutlined = tester.widget<OutlinedButton>(
        find.byType(OutlinedButton),
      );
      final secondaryStyle = secondaryOutlined.style!;
      expect(
        secondaryStyle.minimumSize?.resolve({}),
        const Size(double.infinity, 68),
      );
      expect(secondaryStyle.padding?.resolve({}), const EdgeInsets.all(16));
      final secondaryShape =
          secondaryStyle.shape?.resolve({}) as RoundedRectangleBorder;
      expect(secondaryShape.borderRadius, BorderRadius.circular(14));
    },
  );

  testWidgets('Secondary colours: white fill, KioskColors.text label', (
    tester,
  ) async {
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: SecondaryButton(label: 'Secondary', onPressed: () {}),
        ),
      ),
    );

    final secondaryOutlined = tester.widget<OutlinedButton>(
      find.byType(OutlinedButton),
    );
    final secondaryStyle = secondaryOutlined.style!;
    expect(secondaryStyle.backgroundColor?.resolve({}), Colors.white);
    expect(secondaryStyle.foregroundColor?.resolve({}), KioskColors.text);
  });

  testWidgets(
    'Disabled button: onPressed: null -> Opacity 0.5 and fill still KioskColors.brand',
    (tester) async {
      await tester.pumpWidget(
        const MaterialApp(
          home: Scaffold(
            body: PrimaryButton(label: 'Disabled', onPressed: null),
          ),
        ),
      );

      final opacityFinder = find.byType(Opacity);
      expect(opacityFinder, findsOneWidget);
      final opacityWidget = tester.widget<Opacity>(opacityFinder);
      expect(opacityWidget.opacity, 0.5);

      final filledButton = tester.widget<FilledButton>(
        find.byType(FilledButton),
      );
      final style = filledButton.style!;
      expect(
        style.backgroundColor?.resolve({WidgetState.disabled}),
        KioskColors.brand,
      );
    },
  );

  testWidgets(
    'Quote box and last row: 0xFFFBFAFB, radius 14, padding 18/22; last row 24 and brand; label KioskColors.text',
    (tester) async {
      final l10nObj = _testL10n();
      final quote = QuoteModel(
        identity: const RequestIdentity(
          id: 'q1',
          queue: 'K-001',
          orderNo: 'ORD-001',
          transactionNo: null,
          status: KioskStatus.quoted,
          rawStatus: 'QUOTED',
        ),
        paymentMethod: 'Cash',
        type: ExchangeType.buy,
        currency: 'USD',
        rate: 32.1,
        foreign: 100,
        thb: 3210,
      );

      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: QuoteSummary(l10n: l10nObj, quote: quote),
          ),
        ),
      );

      final containerFinder = find
          .descendant(
            of: find.byType(QuoteSummary),
            matching: find.byType(Container),
          )
          .first;
      final container = tester.widget<Container>(containerFinder);
      final decoration = container.decoration as BoxDecoration;
      expect(decoration.color, const Color(0xFFFBFAFB));
      expect(decoration.borderRadius, BorderRadius.circular(14));
      expect(decoration.border?.top.color, KioskColors.line);
      expect(
        container.padding,
        const EdgeInsets.symmetric(vertical: 18, horizontal: 22),
      );

      final giveLabel = tester.widget<Text>(find.text('You give'));
      expect(giveLabel.style?.fontSize, 18);
      expect(giveLabel.style?.fontWeight, FontWeight.w400);
      expect(giveLabel.style?.color, KioskColors.text);

      final receiveLabel = tester.widget<Text>(find.text('You receive'));
      expect(receiveLabel.style?.fontSize, 24);
      expect(receiveLabel.style?.color, KioskColors.brand);

      final receiveValue = tester.widget<Text>(find.text('3,210.00 THB'));
      expect(receiveValue.style?.fontSize, 24);
      expect(receiveValue.style?.fontWeight, FontWeight.w700);
      expect(receiveValue.style?.color, KioskColors.brand);
    },
  );

  testWidgets('Quote dividers: 3 dividers for 4 rows', (tester) async {
    final l10nObj = _testL10n();
    final quote = QuoteModel(
      identity: const RequestIdentity(
        id: 'q1',
        queue: 'K-001',
        orderNo: 'ORD-001',
        transactionNo: null,
        status: KioskStatus.quoted,
        rawStatus: 'QUOTED',
      ),
      paymentMethod: 'Cash',
      type: ExchangeType.buy,
      currency: 'USD',
      rate: 32.1,
      foreign: 100,
      thb: 3210,
    );

    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: QuoteSummary(l10n: l10nObj, quote: quote),
        ),
      ),
    );

    expect(find.byType(Divider), findsNWidgets(3));
  });

  testWidgets(
    'Identity box and rows: 0xFFF8F7F9, no border, radius 12; text 13; status value brand',
    (tester) async {
      final l10nObj = _testL10n();
      final identity = RequestIdentity(
        id: 'req-1',
        queue: 'K-001',
        orderNo: 'ORD-001',
        transactionNo: 'TXN-001',
        status: KioskStatus.accepted,
        rawStatus: 'ACCEPTED',
      );

      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: IdentityBlock(l10n: l10nObj, identity: identity),
          ),
        ),
      );

      final containerFinder = find
          .descendant(
            of: find.byType(IdentityBlock),
            matching: find.byType(Container),
          )
          .first;
      final container = tester.widget<Container>(containerFinder);
      final decoration = container.decoration as BoxDecoration;
      expect(decoration.color, const Color(0xFFF8F7F9));
      expect(decoration.border, isNull);
      expect(decoration.borderRadius, BorderRadius.circular(12));
      expect(
        container.padding,
        const EdgeInsets.symmetric(vertical: 16, horizontal: 20),
      );

      final orderLabel = tester.widget<Text>(find.text('Order number'));
      expect(orderLabel.style?.fontSize, 13);
      expect(orderLabel.style?.color, KioskColors.text);

      final statusValue = tester.widget<Text>(find.text('ACCEPTED'));
      expect(statusValue.style?.fontSize, 13);
      expect(statusValue.style?.fontWeight, FontWeight.w700);
      expect(statusValue.style?.color, KioskColors.brand);
      expect(statusValue.textAlign, TextAlign.right);
    },
  );

  testWidgets(
    'Progress steps: finished border and text 0xFF18794E; pending border 0xFFDDDDDD, text 0xFF666666; weight w400',
    (tester) async {
      final l10nObj = _testL10n();

      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: OrderProgress(l10n: l10nObj, status: KioskStatus.idChecked),
          ),
        ),
      );

      final finishedText = tester.widget<Text>(find.text('✓ ID Checked'));
      expect(finishedText.style?.color, const Color(0xFF18794E));
      expect(finishedText.style?.fontWeight, FontWeight.w400);
      expect(finishedText.style?.fontSize, 18);

      final finishedBox = tester.widget<Container>(
        find
            .ancestor(
              of: find.text('✓ ID Checked'),
              matching: find.byType(Container),
            )
            .first,
      );
      final finishedDecoration = finishedBox.decoration as BoxDecoration;
      expect(finishedDecoration.border?.top.color, const Color(0xFF18794E));
      expect(finishedDecoration.color, Colors.white);

      final pendingText = tester.widget<Text>(find.text('Success'));
      expect(pendingText.style?.color, const Color(0xFF666666));
      expect(pendingText.style?.fontWeight, FontWeight.w400);
      expect(pendingText.style?.fontSize, 18);

      final pendingBox = tester.widget<Container>(
        find
            .ancestor(
              of: find.text('Success'),
              matching: find.byType(Container),
            )
            .first,
      );
      final pendingDecoration = pendingBox.decoration as BoxDecoration;
      expect(pendingDecoration.border?.top.color, const Color(0xFFDDDDDD));
      expect(pendingDecoration.color, Colors.white);
    },
  );

  testWidgets('Queue number: size 44, w700, brand, tabular figures', (
    tester,
  ) async {
    await tester.pumpWidget(
      const MaterialApp(home: Scaffold(body: QueueNumber('K-001'))),
    );

    final textWidget = tester.widget<Text>(find.text('K-001'));
    final style = textWidget.style!;
    expect(style.fontSize, 44);
    expect(style.fontWeight, FontWeight.w700);
    expect(style.color, KioskColors.brand);
    expect(style.letterSpacing, 2);
    expect(style.fontFeatures, contains(const FontFeature.tabularFigures()));

    final paddingWidget = tester.widget<Padding>(
      find.ancestor(of: find.byType(Ltr), matching: find.byType(Padding)).first,
    );
    expect(paddingWidget.padding, const EdgeInsets.symmetric(vertical: 14));
  });
}
