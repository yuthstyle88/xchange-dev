import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'flutter_thermal_usb_port.dart';
import 'ticket_printer.dart';
import 'usb_printer_port.dart';
import 'usb_ticket_printer.dart';

const String printerIdPreferenceKey = 'queueMachine.printerId';

final usbPrinterPortProvider = Provider<UsbPrinterPort>(
  (ref) => FlutterThermalUsbPort(),
);

class SelectedPrinterIdNotifier extends Notifier<String?> {
  @override
  String? build() {
    _load();
    return null;
  }

  Future<void> _load() async {
    final prefs = await SharedPreferences.getInstance();
    final stored = prefs.getString(printerIdPreferenceKey);
    if (state == null && stored != null) {
      state = stored;
    }
  }

  Future<void> select(String id) async {
    state = id;
    final prefs = await SharedPreferences.getInstance();
    await prefs.setString(printerIdPreferenceKey, id);
  }
}

final selectedPrinterIdProvider =
    NotifierProvider<SelectedPrinterIdNotifier, String?>(
      SelectedPrinterIdNotifier.new,
    );

final ticketPrinterProvider = Provider<TicketPrinter>((ref) {
  final port = ref.watch(usbPrinterPortProvider);
  final deviceId = ref.watch(selectedPrinterIdProvider);
  return UsbTicketPrinter(port, deviceId: deviceId);
});
