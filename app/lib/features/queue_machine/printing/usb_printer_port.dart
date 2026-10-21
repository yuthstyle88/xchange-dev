import 'dart:typed_data';

class UsbPrinterDevice {
  const UsbPrinterDevice({required this.id, required this.name});

  final String id;
  final String name;

  @override
  bool operator ==(Object other) =>
      other is UsbPrinterDevice && other.id == id && other.name == name;

  @override
  int get hashCode => Object.hash(id, name);
}

abstract class UsbPrinterPort {
  Future<List<UsbPrinterDevice>> devices();
  Future<bool> hasPermission(UsbPrinterDevice device);
  Future<bool> requestPermission(UsbPrinterDevice device);
  Future<void> writeRaw(UsbPrinterDevice device, Uint8List bytes);
}
