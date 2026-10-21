import 'dart:typed_data';

abstract class TicketPrinter {
  Future<void> write(Uint8List bytes);
}

class PrinterUnavailable implements Exception {
  const PrinterUnavailable(this.reason);

  final String reason;

  @override
  String toString() => 'PrinterUnavailable($reason)';
}
