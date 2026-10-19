import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/features/kiosk/app/kiosk_providers.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/controllers/kiosk_controller.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/pages/select_page.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/widgets/kiosk_widgets.dart';
import 'package:xchange_kiosk/shared/theme/colors.dart';

import 'kiosk_controller_test.dart' show FakeRepository, l10n;

void main() {
  late FakeRepository server;
  late ProviderContainer container;

  Future<KioskController> pumpSelectPage(
    WidgetTester tester, {
    Size size = const Size(1280, 900),
  }) async {
    tester.view.physicalSize = size;
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);
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
        child: const MaterialApp(
          home: Scaffold(body: SingleChildScrollView(child: SelectPage())),
        ),
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

  setUp(() => server = FakeRepository());

  testWidgets('one form card with radius 24, padding 30, border and shadow', (
    tester,
  ) async {
    await pumpSelectPage(tester);
    expect(find.byType(KioskCard), findsNothing);

    final cards = tester.widgetList<Container>(find.byType(Container)).where((
      c,
    ) {
      final d = c.decoration;
      return d is BoxDecoration &&
          d.borderRadius == BorderRadius.circular(24) &&
          c.padding == const EdgeInsets.all(30);
    }).toList();
    expect(cards, hasLength(1));
    final dec = cards.single.decoration! as BoxDecoration;
    expect(dec.color, Colors.white);
    expect(dec.border, Border.all(color: const Color(0xFFE9E5E9)));
    expect(dec.boxShadow, const [
      BoxShadow(
        color: Color(0x0628202B),
        offset: Offset(0, 12),
        blurRadius: 40,
      ),
    ]);
    await finish(tester);
  });

  testWidgets(
    'columns use flex 21:20 with gap 32 and end divider on wide screens',
    (tester) async {
      await pumpSelectPage(tester);
      final flex21 = tester
          .widgetList<Expanded>(find.byType(Expanded))
          .where((e) => e.flex == 21);
      final flex20 = tester
          .widgetList<Expanded>(find.byType(Expanded))
          .where((e) => e.flex == 20);
      expect(flex21, hasLength(1));
      expect(flex20, hasLength(1));

      final row = tester.widget<Row>(
        find
            .ancestor(
              of: find.byWidget(flex21.single),
              matching: find.byType(Row),
            )
            .first,
      );
      final gap = row.children[1] as SizedBox;
      expect(gap.width, 32);

      final dividerBox = flex21.single.child as Container;
      expect(dividerBox.padding, const EdgeInsetsDirectional.only(end: 30));
      final dec = dividerBox.decoration! as BoxDecoration;
      expect(
        dec.border,
        const BorderDirectional(end: BorderSide(color: KioskColors.line)),
      );
      await finish(tester);
    },
  );

  testWidgets(
    'direction buttons use min-height 90, title 21 w700, gradient and shadow',
    (tester) async {
      await pumpSelectPage(tester);
      final buyTitle = tester.widget<Text>(find.text('buy'));
      expect(buyTitle.style?.fontSize, 21);
      expect(buyTitle.style?.fontWeight, FontWeight.w700);
      expect(buyTitle.style?.color, Colors.white);

      final buyHint = tester.widget<Text>(find.text('buyHint'));
      expect(buyHint.style?.fontSize, 14);
      expect(buyHint.style?.color, const Color(0xFFFFE3ED));

      final sellHint = tester.widget<Text>(find.text('sellHint'));
      expect(sellHint.style?.fontSize, 14);
      expect(sellHint.style?.color, KioskColors.muted);

      final buyContainer = tester
          .widgetList<Container>(
            find.ancestor(
              of: find.text('buy'),
              matching: find.byType(Container),
            ),
          )
          .firstWhere((c) => c.constraints?.minHeight == 90);
      expect(
        buyContainer.padding,
        const EdgeInsets.symmetric(horizontal: 24, vertical: 18),
      );
      final buyDec = buyContainer.decoration! as BoxDecoration;
      final gradient = buyDec.gradient! as LinearGradient;
      expect(gradient.colors, const [Color(0xFFAB0044), Color(0xFF890032)]);
      expect(buyDec.boxShadow, const [
        BoxShadow(
          color: Color(0x1CA5003C),
          offset: Offset(0, 8),
          blurRadius: 24,
        ),
      ]);
      await finish(tester);
    },
  );

  testWidgets(
    'currency chips use min-height 76, selected fill 0xFFFFF0F5, name 0xFF79727A',
    (tester) async {
      final c = await pumpSelectPage(tester);
      c.setCurrency('USD');
      await tester.pump();

      final usdInk = find.byKey(const Key('currency-USD'));
      final jpyInk = find.byKey(const Key('currency-JPY'));
      final usdMaterial = tester.widget<Material>(
        find.ancestor(of: usdInk, matching: find.byType(Material)).first,
      );
      final jpyMaterial = tester.widget<Material>(
        find.ancestor(of: jpyInk, matching: find.byType(Material)).first,
      );
      expect(usdMaterial.color, const Color(0xFFFFF0F5));
      expect(jpyMaterial.color, Colors.white);

      final chipContainer = tester.widget<Container>(
        find.descendant(of: usdInk, matching: find.byType(Container)).first,
      );
      expect(chipContainer.constraints?.minHeight, 76);
      expect(
        chipContainer.padding,
        const EdgeInsets.symmetric(horizontal: 6, vertical: 12),
      );

      final jpyTexts = tester
          .widgetList<Text>(
            find.descendant(of: jpyInk, matching: find.byType(Text)),
          )
          .toList();
      expect(jpyTexts.last.style?.fontSize, 12);
      expect(jpyTexts.last.style?.color, const Color(0xFF79727A));

      final usdTexts = tester
          .widgetList<Text>(
            find.descendant(of: usdInk, matching: find.byType(Text)),
          )
          .toList();
      expect(usdTexts.last.style?.fontSize, 12);
      expect(usdTexts.last.style?.color, KioskColors.brand);
      await finish(tester);
    },
  );

  testWidgets('rate box uses 0xFFF5F5F7 fill and value 20 w700', (
    tester,
  ) async {
    final c = await pumpSelectPage(tester);
    c.setCurrency('USD');
    await tester.pump();

    final rateLabel = find.text('weBuy');
    final rateLabelWidget = tester.widget<Text>(rateLabel);
    expect(rateLabelWidget.style?.fontSize, 12);
    expect(rateLabelWidget.style?.color, KioskColors.muted);

    final rateBox = tester
        .widgetList<Container>(
          find.ancestor(of: rateLabel, matching: find.byType(Container)),
        )
        .firstWhere((box) {
          final d = box.decoration;
          return d is BoxDecoration && d.color == const Color(0xFFF5F5F7);
        });
    final rateDec = rateBox.decoration! as BoxDecoration;
    expect(rateDec.borderRadius, BorderRadius.circular(12));
    expect(rateDec.border, isNull);

    final rateLtr = tester.widget<Ltr>(
      find
          .descendant(of: find.byWidget(rateBox), matching: find.byType(Ltr))
          .first,
    );
    expect(rateLtr.style?.fontSize, 20);
    expect(rateLtr.style?.fontWeight, FontWeight.w700);
    expect(rateLtr.style?.color, KioskColors.ink);
    await finish(tester);
  });

  testWidgets(
    'amount field and receive box match v2 sizes, fills and borders',
    (tester) async {
      final c = await pumpSelectPage(tester);
      c
        ..setCurrency('USD')
        ..setAmount('100');
      await tester.pump();

      final amountField = tester.widget<TextField>(
        find.byKey(const Key('amount')),
      );
      expect(amountField.style?.fontSize, 30);
      expect(amountField.style?.fontWeight, FontWeight.w600);
      final enabledBorder =
          amountField.decoration?.enabledBorder as OutlineInputBorder?;
      expect(
        enabledBorder?.borderSide,
        const BorderSide(color: Color(0xFFD9D4DB), width: 2),
      );
      expect(enabledBorder?.borderRadius, BorderRadius.circular(10));

      final receiveBoxes = tester
          .widgetList<Container>(find.byType(Container))
          .where((box) {
            final d = box.decoration;
            return d is BoxDecoration && d.color == const Color(0xFFFFF3F7);
          })
          .toList();
      expect(receiveBoxes, hasLength(1));
      final receiveDec = receiveBoxes.single.decoration! as BoxDecoration;
      expect(
        receiveDec.border,
        Border.all(color: const Color(0xFFF1D5DF), width: 2),
      );
      expect(receiveDec.borderRadius, BorderRadius.circular(10));

      final receiveLtr = tester.widget<Ltr>(
        find.descendant(
          of: find.byWidget(receiveBoxes.single),
          matching: find.byType(Ltr),
        ),
      );
      expect(receiveLtr.style?.fontSize, 28);
      expect(receiveLtr.style?.fontWeight, FontWeight.w700);
      expect(receiveLtr.style?.color, KioskColors.brand);
      await finish(tester);
    },
  );
}
