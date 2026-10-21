// QM-3 (kiosk-shared-phone §5r F5, rows A3, A4, A8, A9): the queue machine writes the server's ESC/POS bytes,
// unchanged, to the POS80 on USB (owner Q-F1). Written before the implementation by the sub-controller
// claude/36d9dc6a. Do not edit, delete or weaken these tests; add tests in a file of your own.
//
// What these tests pin (the names QM-3 implements):
// - `lib/features/queue_machine/printing/ticket_printer.dart`:
//     abstract class TicketPrinter { Future<void> write(Uint8List bytes); }
//     class PrinterUnavailable implements Exception { const PrinterUnavailable(this.reason); final String reason; }
//       reason is one of 'noDevice', 'permissionRefused', 'writeFailed'.
// - `lib/features/queue_machine/printing/usb_printer_port.dart` (the one seam the plugin sits behind):
//     class UsbPrinterDevice { const UsbPrinterDevice({required this.id, required this.name}); final String id, name; }
//     abstract class UsbPrinterPort {
//       Future<List<UsbPrinterDevice>> devices();
//       Future<bool> hasPermission(UsbPrinterDevice device);
//       Future<bool> requestPermission(UsbPrinterDevice device);
//       Future<void> writeRaw(UsbPrinterDevice device, Uint8List bytes);
//     }
//   The real implementation (`flutter_thermal_printer`) lives in its own file and is not exercised here.
// - `lib/features/queue_machine/printing/usb_ticket_printer.dart`:
//     class UsbTicketPrinter implements TicketPrinter { UsbTicketPrinter(UsbPrinterPort port, {String? deviceId}); }
//   It writes to the device with `deviceId`, or to the first device when none is chosen; asks for the USB
//   permission only when it is not already held; and turns every failure into a PrinterUnavailable.
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/features/queue_machine/printing/ticket_printer.dart';
import 'package:xchange_kiosk/features/queue_machine/printing/usb_printer_port.dart';
import 'package:xchange_kiosk/features/queue_machine/printing/usb_ticket_printer.dart';

class _FakePort implements UsbPrinterPort {
  _FakePort(this.attached, {this.grant = true, this.failWrite = false});

  final List<UsbPrinterDevice> attached;
  final bool grant;
  final bool failWrite;
  final Set<String> granted = {};
  int permissionRequests = 0;
  final writes = <({String deviceId, Uint8List bytes})>[];

  @override
  Future<List<UsbPrinterDevice>> devices() async => attached;

  @override
  Future<bool> hasPermission(UsbPrinterDevice device) async =>
      granted.contains(device.id);

  @override
  Future<bool> requestPermission(UsbPrinterDevice device) async {
    permissionRequests += 1;
    if (grant) granted.add(device.id);
    return grant;
  }

  @override
  Future<void> writeRaw(UsbPrinterDevice device, Uint8List bytes) async {
    if (failWrite) throw StateError('USB write failed');
    writes.add((deviceId: device.id, bytes: Uint8List.fromList(bytes)));
  }
}

const _pos80 = UsbPrinterDevice(id: 'usb-1', name: 'POS80');
const _other = UsbPrinterDevice(id: 'usb-2', name: 'Other');

// ESC @, "K042", LF, GS V 0: a small ticket; the printer must receive exactly these bytes.
final _ticket = Uint8List.fromList([
  0x1B,
  0x40,
  0x4B,
  0x30,
  0x34,
  0x32,
  0x0A,
  0x1D,
  0x56,
  0x00,
]);

Matcher _unavailable(String reason) =>
    isA<PrinterUnavailable>().having((e) => e.reason, 'reason', reason);

void main() {
  test(
    'A3: the server bytes reach the printer unchanged, byte for byte',
    () async {
      final port = _FakePort([_pos80])..granted.add(_pos80.id);
      final TicketPrinter printer = UsbTicketPrinter(port);
      await printer.write(_ticket);
      expect(port.writes, hasLength(1));
      expect(port.writes.single.deviceId, 'usb-1');
      expect(port.writes.single.bytes, orderedEquals(_ticket));
    },
  );

  test('the chosen device is used when several are attached', () async {
    final port = _FakePort([_other, _pos80])
      ..granted.addAll({'usb-1', 'usb-2'});
    await UsbTicketPrinter(port, deviceId: 'usb-1').write(_ticket);
    expect(port.writes.single.deviceId, 'usb-1');
  });

  test(
    'A8: no USB printer attached => PrinterUnavailable(noDevice), nothing written',
    () async {
      final port = _FakePort([]);
      await expectLater(
        UsbTicketPrinter(port).write(_ticket),
        throwsA(_unavailable('noDevice')),
      );
      expect(port.writes, isEmpty);
    },
  );

  test(
    'A8: the chosen device is gone => PrinterUnavailable(noDevice)',
    () async {
      final port = _FakePort([_other])..granted.add('usb-2');
      await expectLater(
        UsbTicketPrinter(port, deviceId: 'usb-1').write(_ticket),
        throwsA(_unavailable('noDevice')),
      );
      expect(port.writes, isEmpty);
    },
  );

  test(
    'A8: the Android USB permission refused => PrinterUnavailable(permissionRefused), nothing written',
    () async {
      final port = _FakePort([_pos80], grant: false);
      await expectLater(
        UsbTicketPrinter(port).write(_ticket),
        throwsA(_unavailable('permissionRefused')),
      );
      expect(port.permissionRequests, 1);
      expect(port.writes, isEmpty);
    },
  );

  test(
    'A9: the permission is asked once; the next ticket writes without asking again',
    () async {
      final port = _FakePort([_pos80]);
      final printer = UsbTicketPrinter(port);
      await printer.write(_ticket);
      await printer.write(_ticket);
      expect(port.permissionRequests, 1);
      expect(port.writes, hasLength(2));
    },
  );

  test(
    'A4: a failed write => PrinterUnavailable(writeFailed); writing the same bytes again works',
    () async {
      final failing = _FakePort([_pos80], failWrite: true)
        ..granted.add(_pos80.id);
      await expectLater(
        UsbTicketPrinter(failing).write(_ticket),
        throwsA(_unavailable('writeFailed')),
      );

      final working = _FakePort([_pos80])..granted.add(_pos80.id);
      await UsbTicketPrinter(working).write(_ticket);
      expect(working.writes.single.bytes, orderedEquals(_ticket));
    },
  );
}
