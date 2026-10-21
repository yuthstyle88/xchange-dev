import 'dart:async';
import 'dart:typed_data';

import 'package:flutter_thermal_printer/flutter_thermal_printer.dart';
import 'package:flutter_thermal_printer/printer_manager.dart';
import 'package:flutter_thermal_printer/utils/printer.dart';

import 'usb_printer_port.dart';

class FlutterThermalUsbPort implements UsbPrinterPort {
  FlutterThermalUsbPort({FlutterThermalPrinter? plugin})
    : _plugin = plugin ?? FlutterThermalPrinter.instance;

  final FlutterThermalPrinter _plugin;
  final Map<String, Printer> _printersById = {};

  static String _idFor(Printer printer) {
    final vendor = printer.vendorId ?? '';
    final product = printer.productId ?? '';
    if (vendor.isNotEmpty || product.isNotEmpty) {
      return '$vendor:$product';
    }
    return printer.address ?? printer.name ?? '';
  }

  Printer _printerFor(UsbPrinterDevice device) {
    final cached = _printersById[device.id];
    if (cached != null) return cached;
    final parts = device.id.split(':');
    final vendorId = parts.isNotEmpty ? parts.first : device.id;
    final productId = parts.length > 1 ? parts[1] : '';
    return Printer(
      name: device.name,
      address: vendorId,
      vendorId: vendorId,
      productId: productId,
      connectionType: ConnectionType.USB,
    );
  }

  @override
  Future<List<UsbPrinterDevice>> devices() async {
    var latest = const <Printer>[];
    final sub = _plugin.devicesStream.listen((list) {
      latest = list;
    });
    try {
      await _plugin.getPrinters(connectionTypes: const [ConnectionType.USB]);
    } finally {
      await sub.cancel();
      await _plugin.stopScan();
    }

    _printersById.clear();
    final result = <UsbPrinterDevice>[];
    for (final printer in latest) {
      if (printer.connectionType != ConnectionType.USB) continue;
      final id = _idFor(printer);
      if (id.isEmpty) continue;
      _printersById[id] = printer;
      result.add(UsbPrinterDevice(id: id, name: printer.name ?? id));
    }
    return result;
  }

  @override
  Future<bool> hasPermission(UsbPrinterDevice device) =>
      PrinterManager.instance.isConnected(_printerFor(device));

  @override
  Future<bool> requestPermission(UsbPrinterDevice device) =>
      _plugin.connect(_printerFor(device));

  @override
  Future<void> writeRaw(UsbPrinterDevice device, Uint8List bytes) =>
      _plugin.printData(_printerFor(device), bytes);
}
