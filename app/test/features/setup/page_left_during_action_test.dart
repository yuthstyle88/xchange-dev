// #174 (A3, #51 sixth finding): a share or a link that fails after its page
// has gone must raise nothing. The pages read `ref` inside the `catch`, after
// an `await`; once the page is disposed Riverpod 3 throws there.
// Written before the code.
//
// Mutation that must turn this red: read the controller inside the catch again.
import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/core/platform/device_actions.dart';
import 'package:xchange_kiosk/features/kiosk/app/kiosk_providers.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/controllers/kiosk_controller.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/pages/kiosk_page.dart';

import '../kiosk/kiosk_controller_test.dart' show FakeRepository, l10n;

/// A 1×1 transparent PNG.
const _png =
    'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlE'
    'QVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==';

/// Holds every action open until the test lets it fail.
class _SlowActions extends DeviceActions {
  final pending = <Completer<void>>[];

  @override
  List<String> languages() => const ['en'];

  Future<void> _wait() {
    final c = Completer<void>();
    pending.add(c);
    return c.future;
  }

  @override
  Future<void> saveImage(String dataUrl, String filename) => _wait();

  @override
  Future<void> openExternal(String url) => _wait();

  @override
  Future<void> openHttps(Uri uri) => _wait();
}

void main() {
  late FakeRepository server;
  late _SlowActions actions;
  late ProviderContainer container;

  Future<KioskController> toWaiting(WidgetTester tester) async {
    tester.view.physicalSize = const Size(1280, 1600);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);
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
    return c;
  }

  Future<void> leaveAndFail(WidgetTester tester) async {
    expect(actions.pending, hasLength(1));
    await tester.pumpWidget(const SizedBox());
    actions.pending.single.completeError(
      const DeviceActionException('no handler'),
    );
    await tester.pump();
    await tester.pump();
    expect(tester.takeException(), isNull);
    container.dispose();
  }

  setUp(() {
    server = FakeRepository();
    actions = _SlowActions();
  });

  testWidgets('the Telegram link failing after the page has gone', (
    tester,
  ) async {
    await toWaiting(tester);
    final link = find.text('@XCHANGEXTK');
    await tester.ensureVisible(link);
    await tester.tap(link);
    await tester.pump();
    await leaveAndFail(tester);
  });

  testWidgets('saving the receipt failing after the page has gone', (
    tester,
  ) async {
    server.receiptImage = _png;
    await toWaiting(tester);
    await tester.pump(const Duration(seconds: 3));
    await tester.pump();
    expect(container.read(kioskControllerProvider).screen, KioskScreen.receipt);
    final save = find.byKey(const Key('save-receipt'));
    await tester.ensureVisible(save);
    await tester.tap(save);
    await tester.pump();
    await leaveAndFail(tester);
  });

  testWidgets('while the page is still there, a failure shows the error line', (
    tester,
  ) async {
    await toWaiting(tester);
    final link = find.text('@XCHANGEXTK');
    await tester.ensureVisible(link);
    await tester.tap(link);
    await tester.pump();
    actions.pending.single.completeError(
      const DeviceActionException('no handler'),
    );
    await tester.pump();
    expect(container.read(kioskControllerProvider).error, 'generic-en');
    await tester.pumpWidget(const SizedBox());
    container.dispose();
  });
}
