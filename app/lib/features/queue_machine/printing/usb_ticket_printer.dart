import 'dart:typed_data';

import 'ticket_printer.dart';
import 'usb_printer_port.dart';

class UsbTicketPrinter implements TicketPrinter {
  UsbTicketPrinter(this._port, {this.deviceId});

  final UsbPrinterPort _port;
  final String? deviceId;

  @override
  Future<void> write(Uint8List bytes) async {
    final List<UsbPrinterDevice> attached;
    try {
      attached = await _port.devices();
    } catch (_) {
      throw const PrinterUnavailable('noDevice');
    }

    final UsbPrinterDevice? target = deviceId == null
        ? (attached.isEmpty ? null : attached.first)
        : attached.where((d) => d.id == deviceId).firstOrNull;
    if (target == null) {
      throw const PrinterUnavailable('noDevice');
    }

    final bool permitted;
    try {
      permitted =
          await _port.hasPermission(target) ||
          await _port.requestPermission(target);
    } catch (_) {
      throw const PrinterUnavailable('permissionRefused');
    }
    if (!permitted) {
      throw const PrinterUnavailable('permissionRefused');
    }

    try {
      await _port.writeRaw(target, bytes);
    } catch (_) {
      throw const PrinterUnavailable('writeFailed');
    }
  }
}
