import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../../../app/app.dart';
import '../../../../shared/theme/colors.dart';
import '../../printing/printer_providers.dart';
import '../../printing/ticket_printer.dart';
import '../../printing/usb_printer_port.dart';
import 'queue_machine_home_page.dart';

final Uint8List testPrintBytes = Uint8List.fromList([
  0x1B,
  0x40,
  0x54,
  0x45,
  0x53,
  0x54,
  0x0A,
  0x1D,
  0x56,
  0x00,
]);

class PrinterPage extends ConsumerStatefulWidget {
  const PrinterPage({super.key});

  @override
  ConsumerState<PrinterPage> createState() => _PrinterPageState();
}

class _PrinterPageState extends ConsumerState<PrinterPage> {
  static const Map<String, String> _fallbackTh = {
    'printer_title': 'ตั้งค่าเครื่องพิมพ์',
    'printer_no_devices': 'ไม่พบเครื่องพิมพ์ที่เชื่อมต่อ',
    'printer_refresh': 'ค้นหาเครื่องพิมพ์',
    'printer_test_print': 'ทดสอบพิมพ์',
    'printer_test_ok': 'ส่งคำสั่งพิมพ์ทดสอบแล้ว',
    'printer_error_no_device': 'ไม่พบเครื่องพิมพ์',
    'printer_error_permission': 'ไม่ได้รับอนุญาตให้ใช้เครื่องพิมพ์',
    'printer_error_write': 'พิมพ์ไม่สำเร็จ กรุณาลองใหม่',
  };

  List<UsbPrinterDevice> _devices = const [];
  String? _selectedId;
  String? _statusKey;
  String? _errorKey;
  bool _printing = false;

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    final port = ref.read(usbPrinterPortProvider);
    final prefs = await SharedPreferences.getInstance();
    final stored = prefs.getString(printerIdPreferenceKey);
    if (stored != null && ref.read(selectedPrinterIdProvider) != stored) {
      await ref.read(selectedPrinterIdProvider.notifier).select(stored);
    }
    List<UsbPrinterDevice> devices = const [];
    try {
      devices = await port.devices();
    } catch (_) {
      devices = const [];
    }
    if (!mounted) return;
    setState(() {
      _devices = devices;
      _selectedId = stored ?? ref.read(selectedPrinterIdProvider);
    });
  }

  Future<void> _selectDevice(String id) async {
    setState(() {
      _selectedId = id;
      _statusKey = null;
      _errorKey = null;
    });
    await ref.read(selectedPrinterIdProvider.notifier).select(id);
  }

  Future<void> _testPrint() async {
    setState(() {
      _printing = true;
      _statusKey = null;
      _errorKey = null;
    });
    try {
      await ref.read(ticketPrinterProvider).write(testPrintBytes);
      if (!mounted) return;
      setState(() {
        _printing = false;
        _statusKey = 'printer_test_ok';
      });
    } on PrinterUnavailable catch (e) {
      if (!mounted) return;
      setState(() {
        _printing = false;
        _errorKey = switch (e.reason) {
          'noDevice' => 'printer_error_no_device',
          'permissionRefused' => 'printer_error_permission',
          _ => 'printer_error_write',
        };
      });
    } catch (_) {
      if (!mounted) return;
      setState(() {
        _printing = false;
        _errorKey = 'printer_error_write';
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final lang = ref.watch(appLanguageProvider);
    final l10nAsync = ref.watch(queueMachineL10nProvider);

    String tr(String key) => l10nAsync.when(
      data: (l) => l.text(lang, key),
      loading: () => _fallbackTh[key] ?? key,
      error: (_, _) => _fallbackTh[key] ?? key,
    );

    return Scaffold(
      backgroundColor: Colors.white,
      body: SafeArea(
        child: Center(
          child: SingleChildScrollView(
            padding: const EdgeInsets.all(24),
            child: ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: 520),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Text(
                    tr('printer_title'),
                    key: const Key('printer-title'),
                    style: const TextStyle(
                      fontSize: 26,
                      fontWeight: FontWeight.w800,
                    ),
                  ),
                  const SizedBox(height: 16),
                  if (_devices.isEmpty)
                    Text(
                      tr('printer_no_devices'),
                      key: const Key('printer-no-devices'),
                      style: const TextStyle(fontSize: 16),
                    )
                  else
                    for (final device in _devices)
                      ListTile(
                        key: Key('printer-device-${device.id}'),
                        leading: Icon(
                          _selectedId == device.id
                              ? Icons.radio_button_checked
                              : Icons.radio_button_unchecked,
                          color: _selectedId == device.id
                              ? KioskColors.brand
                              : null,
                        ),
                        title: Text(device.name),
                        subtitle: Text(device.id),
                        selected: _selectedId == device.id,
                        onTap: () => _selectDevice(device.id),
                      ),
                  const SizedBox(height: 16),
                  OutlinedButton(
                    key: const Key('printer-refresh'),
                    onPressed: _load,
                    child: Text(tr('printer_refresh')),
                  ),
                  const SizedBox(height: 12),
                  FilledButton(
                    key: const Key('printer-test-print'),
                    onPressed: _printing ? null : _testPrint,
                    child: Text(tr('printer_test_print')),
                  ),
                  if (_statusKey != null) ...[
                    const SizedBox(height: 12),
                    Text(
                      tr(_statusKey!),
                      key: const Key('printer-status'),
                      style: const TextStyle(
                        fontSize: 16,
                        fontWeight: FontWeight.w700,
                        color: KioskColors.brand,
                      ),
                    ),
                  ],
                  if (_errorKey != null) ...[
                    const SizedBox(height: 12),
                    Text(
                      tr(_errorKey!),
                      key: const Key('printer-error'),
                      style: const TextStyle(fontSize: 16, color: Colors.red),
                    ),
                  ],
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}
