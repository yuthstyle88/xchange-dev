// #2478: Kiosk receipt Save image button in-flight guard.
import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/core/platform/device_actions.dart';
import 'package:xchange_kiosk/features/kiosk/app/kiosk_providers.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/controllers/kiosk_controller.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/pages/kiosk_page.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/widgets/kiosk_widgets.dart';

import 'kiosk_controller_test.dart' show FakeRepository, l10n;

/// A 1×1 transparent PNG.
const _png =
    'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlE'
    'QVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==';

class _HoldSaveActions extends DeviceActions {
  final pending = <Completer<void>>[];
  final savedFilenames = <String>[];

  @override
  List<String> languages() => const ['en'];

  @override
  Future<void> saveImage(String dataUrl, String filename) {
    savedFilenames.add(filename);
    final c = Completer<void>();
    pending.add(c);
    return c.future;
  }

  @override
  Future<void> openExternal(String url) async {}

  @override
  Future<void> openHttps(Uri uri) async {}
}

void main() {
  late FakeRepository server;
  late _HoldSaveActions actions;
  late ProviderContainer container;

  Future<void> toReceipt(WidgetTester tester) async {
    tester.view.physicalSize = const Size(1280, 1600);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);
    server.receiptImage = _png;
    container = ProviderContainer(
      overrides: [
        kioskRepositoryProvider.overrideWithValue(server),
        kioskL10nProvider.overrideWithValue(l10n()),
        deviceLanguagesProvider.overrideWithValue(const ['en']),
        deviceActionsProvider.overrideWithValue(actions),
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
    c
      ..setCurrency('USD')
      ..setAmount('100')
      ..setPaymentMethod('Cash');
    await c.submitQuote();
    await tester.pump(const Duration(seconds: 3));
    await c.confirm();
    await tester.pump();
    expect(container.read(kioskControllerProvider).screen, KioskScreen.waiting);
    await tester.pump(const Duration(seconds: 3));
    await tester.pump();
    expect(container.read(kioskControllerProvider).screen, KioskScreen.receipt);
  }

  setUp(() {
    server = FakeRepository();
    actions = _HoldSaveActions();
  });

  testWidgets('row 1: Double tap Save image -> saveImage is called once', (
    tester,
  ) async {
    await toReceipt(tester);
    final save = find.byKey(const Key('save-receipt'));
    await tester.ensureVisible(save);

    // Double tap before the first save completes (same frame + next frame)
    await tester.tap(save);
    await tester.tap(save);
    await tester.pump();
    await tester.tap(save);
    await tester.pump();

    expect(actions.pending, hasLength(1));
    expect(tester.widget<PrimaryButton>(save).onPressed, isNull);

    actions.pending.single.complete();
    await tester.pump();

    expect(actions.pending, hasLength(1));
    await tester.pumpWidget(const SizedBox());
    container.dispose();
  });

  testWidgets(
    'row 2: Save fails with DeviceActionException -> errGeneric reported once; button enabled again',
    (tester) async {
      await toReceipt(tester);
      final save = find.byKey(const Key('save-receipt'));
      await tester.ensureVisible(save);

      var errorNotifications = 0;
      final sub = container.listen<String>(
        kioskControllerProvider.select((s) => s.error),
        (prev, next) {
          if (next.isNotEmpty) errorNotifications++;
        },
      );
      addTearDown(sub.close);

      await tester.tap(save);
      await tester.tap(save);
      await tester.pump();

      expect(actions.pending, hasLength(1));
      expect(tester.widget<PrimaryButton>(save).onPressed, isNull);

      actions.pending.single.completeError(
        const DeviceActionException('share rejected'),
      );
      await tester.pump();

      expect(container.read(kioskControllerProvider).error, 'generic-en');
      expect(errorNotifications, 1);
      expect(tester.widget<PrimaryButton>(save).onPressed, isNotNull);

      // Button is enabled again and can start a new save on a subsequent tap
      await tester.tap(save);
      await tester.pump();
      expect(actions.pending, hasLength(2));

      actions.pending.last.complete();
      await tester.pump();
      await tester.pumpWidget(const SizedBox());
      container.dispose();
    },
  );

  testWidgets('row 3: Save succeeds -> Button enabled again', (tester) async {
    await toReceipt(tester);
    final save = find.byKey(const Key('save-receipt'));
    await tester.ensureVisible(save);

    expect(tester.widget<PrimaryButton>(save).onPressed, isNotNull);

    await tester.tap(save);
    await tester.pump();

    // Disabled while save is in flight
    expect(tester.widget<PrimaryButton>(save).onPressed, isNull);

    actions.pending.single.complete();
    await tester.pump();

    // Enabled again once save succeeds
    expect(tester.widget<PrimaryButton>(save).onPressed, isNotNull);
    expect(container.read(kioskControllerProvider).error, isEmpty);

    await tester.tap(save);
    await tester.pump();
    expect(actions.pending, hasLength(2));

    actions.pending.last.complete();
    await tester.pump();
    await tester.pumpWidget(const SizedBox());
    container.dispose();
  });

  testWidgets(
    'row 4: Page left during a failing save after double tap raises nothing',
    (tester) async {
      await toReceipt(tester);
      final save = find.byKey(const Key('save-receipt'));
      await tester.ensureVisible(save);

      await tester.tap(save);
      await tester.tap(save);
      await tester.pump();
      expect(actions.pending, hasLength(1));

      await tester.pumpWidget(const SizedBox());
      actions.pending.single.completeError(
        const DeviceActionException('no handler'),
      );
      await tester.pump();
      await tester.pump();

      expect(tester.takeException(), isNull);
      expect(container.read(kioskControllerProvider).error, 'generic-en');
      container.dispose();
    },
  );
}
