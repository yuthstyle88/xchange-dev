import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:xchange_kiosk/features/queue_machine/presentation/pages/printer_page.dart';
import 'package:xchange_kiosk/features/queue_machine/printing/printer_providers.dart';
import 'package:xchange_kiosk/features/queue_machine/printing/usb_printer_port.dart';

class _FakePort implements UsbPrinterPort {
  _FakePort(this.attached, {this.failWrite = false});

  final List<UsbPrinterDevice> attached;
  final bool failWrite;
  final writes = <({String deviceId, Uint8List bytes})>[];

  @override
  Future<List<UsbPrinterDevice>> devices() async => attached;

  @override
  Future<bool> hasPermission(UsbPrinterDevice device) async => true;

  @override
  Future<bool> requestPermission(UsbPrinterDevice device) async => true;

  @override
  Future<void> writeRaw(UsbPrinterDevice device, Uint8List bytes) async {
    if (failWrite) throw StateError('USB write failed');
    writes.add((deviceId: device.id, bytes: Uint8List.fromList(bytes)));
  }
}

const _pos80 = UsbPrinterDevice(id: 'usb-1', name: 'POS80');
const _other = UsbPrinterDevice(id: 'usb-2', name: 'Other Printer');

void main() {
  testWidgets(
    'lists attached USB devices, stores choice under queueMachine.printerId, and test prints on ทดสอบพิมพ์',
    (tester) async {
      SharedPreferences.setMockInitialValues({});
      final port = _FakePort([_pos80, _other]);

      await tester.pumpWidget(
        ProviderScope(
          overrides: [usbPrinterPortProvider.overrideWithValue(port)],
          child: const MaterialApp(home: PrinterPage()),
        ),
      );
      await tester.pumpAndSettle();

      expect(find.text('POS80'), findsOneWidget);
      expect(find.text('Other Printer'), findsOneWidget);
      expect(find.text('ทดสอบพิมพ์'), findsOneWidget);

      await tester.tap(find.byKey(const Key('printer-device-usb-2')));
      await tester.pumpAndSettle();

      final prefs = await SharedPreferences.getInstance();
      expect(prefs.getString('queueMachine.printerId'), 'usb-2');

      await tester.tap(find.text('ทดสอบพิมพ์'));
      await tester.pumpAndSettle();

      expect(port.writes, hasLength(1));
      expect(port.writes.single.deviceId, 'usb-2');
      expect(port.writes.single.bytes, isNotEmpty);
      expect(find.byKey(const Key('printer-status')), findsOneWidget);
    },
  );

  testWidgets(
    'ticketPrinterProvider uses stored queueMachine.printerId from shared_preferences',
    (tester) async {
      SharedPreferences.setMockInitialValues({
        'queueMachine.printerId': 'usb-2',
      });
      final port = _FakePort([_pos80, _other]);

      await tester.pumpWidget(
        ProviderScope(
          overrides: [usbPrinterPortProvider.overrideWithValue(port)],
          child: const MaterialApp(home: PrinterPage()),
        ),
      );
      await tester.pumpAndSettle();

      await tester.tap(find.byKey(const Key('printer-test-print')));
      await tester.pumpAndSettle();

      expect(port.writes, hasLength(1));
      expect(port.writes.single.deviceId, 'usb-2');
    },
  );

  testWidgets('shows error when test print fails', (tester) async {
    SharedPreferences.setMockInitialValues({});
    final port = _FakePort([_pos80], failWrite: true);

    await tester.pumpWidget(
      ProviderScope(
        overrides: [usbPrinterPortProvider.overrideWithValue(port)],
        child: const MaterialApp(home: PrinterPage()),
      ),
    );
    await tester.pumpAndSettle();

    await tester.tap(find.text('ทดสอบพิมพ์'));
    await tester.pumpAndSettle();

    expect(find.byKey(const Key('printer-error')), findsOneWidget);
  });
}
