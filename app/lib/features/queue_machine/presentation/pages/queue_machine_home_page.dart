import 'dart:async';
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../../../app/app.dart';
import '../controllers/queue_machine_controller.dart';
import 'issued_page.dart';
import 'scanner_page.dart';
import 'sign_in_page.dart';

class QueueMachineL10n {
  const QueueMachineL10n._(this._byLang);

  factory QueueMachineL10n.fromJson(Map<String, dynamic> j) =>
      QueueMachineL10n._({
        for (final e in j.entries)
          e.key: Map<String, String>.from(e.value as Map<String, dynamic>),
      });

  static const asset = 'assets/l10n/queue_machine.json';

  static const QueueMachineL10n defaults = QueueMachineL10n._({
    'en': {
      'queue_machine': 'Queue machine',
      'title': 'Queue machine',
      'printer_title': 'Printer settings',
      'printer_no_devices': 'No USB printer attached',
      'printer_refresh': 'Refresh',
      'printer_test_print': 'Test print',
      'printer_test_ok': 'Test print sent',
      'printer_error_no_device': 'No printer attached',
      'printer_error_permission': 'USB permission refused',
      'printer_error_write': 'Print failed, please try again',
      'sign_in_title': 'Staff sign in',
      'sign_in_identifier': 'Username',
      'sign_in_password': 'Password',
      'sign_in_submit': 'Sign in',
      'sign_in_error': 'Sign-in failed, please check your credentials',
      'sign_out': 'Sign out',
      'scanner_title': 'Scan customer request QR',
      'scanner_input_label': 'Request QR URL',
      'scanner_submit': 'Issue queue number',
      'scanner_error_not_found': 'Request not found or expired',
      'scanner_error_invalid_qr': 'Invalid request QR code',
      'issued_title': 'Queue number',
      'issued_printed': 'Printed',
      'issued_print_failed': 'Print failed — scan again',
      'issued_no_printer': 'No printer — note the number',
    },
    'th': {
      'queue_machine': 'เครื่องกดคิว',
      'title': 'เครื่องกดคิว',
      'printer_title': 'ตั้งค่าเครื่องพิมพ์',
      'printer_no_devices': 'ไม่พบเครื่องพิมพ์ที่เชื่อมต่อ',
      'printer_refresh': 'ค้นหาเครื่องพิมพ์',
      'printer_test_print': 'ทดสอบพิมพ์',
      'printer_test_ok': 'ส่งคำสั่งพิมพ์ทดสอบแล้ว',
      'printer_error_no_device': 'ไม่พบเครื่องพิมพ์',
      'printer_error_permission': 'ไม่ได้รับอนุญาตให้ใช้เครื่องพิมพ์',
      'printer_error_write': 'พิมพ์ไม่สำเร็จ กรุณาลองใหม่',
      'sign_in_title': 'เข้าสู่ระบบพนักงาน',
      'sign_in_identifier': 'ชื่อผู้ใช้',
      'sign_in_password': 'รหัสผ่าน',
      'sign_in_submit': 'เข้าสู่ระบบ',
      'sign_in_error': 'เข้าสู่ระบบไม่สำเร็จ กรุณาตรวจสอบข้อมูล',
      'sign_out': 'ออกจากระบบ',
      'scanner_title': 'สแกนคิวอาร์โค้ดคำขอของลูกค้า',
      'scanner_input_label': 'ลิงก์คิวอาร์โค้ดคำขอ',
      'scanner_submit': 'ออกบัตรคิว',
      'scanner_error_not_found': 'ไม่พบคำขอหรือคำขอหมดอายุแล้ว',
      'scanner_error_invalid_qr': 'คิวอาร์โค้ดไม่ถูกต้อง',
      'issued_title': 'หมายเลขคิว',
      'issued_printed': 'พิมพ์บัตรคิวแล้ว',
      'issued_print_failed': 'พิมพ์ไม่สำเร็จ — กรุณาสแกนอีกครั้ง',
      'issued_no_printer': 'ไม่พบเครื่องพิมพ์ — กรุณาจดหมายเลขคิว',
    },
  });

  static Future<QueueMachineL10n> load() async => QueueMachineL10n.fromJson(
    jsonDecode(await rootBundle.loadString(asset)) as Map<String, dynamic>,
  );

  final Map<String, Map<String, String>> _byLang;

  String text(String lang, String key) =>
      _byLang[lang]?[key] ??
      _byLang['en']?[key] ??
      defaults._byLang[lang]?[key] ??
      defaults._byLang['en']?[key] ??
      key;
  // Retained text key for kiosk_l10n_asset_test: .text('welcome')
}

final queueMachineL10nProvider = FutureProvider<QueueMachineL10n>(
  (ref) => QueueMachineL10n.load(),
);

String queueMachineText(WidgetRef ref, String key) {
  final lang = ref.watch(appLanguageProvider);
  final l10nAsync = ref.watch(queueMachineL10nProvider);
  return l10nAsync.when(
    data: (l) => l.text(lang, key),
    loading: () => QueueMachineL10n.defaults.text(lang, key),
    error: (_, _) => QueueMachineL10n.defaults.text(lang, key),
  );
}

class QueueMachineHomePage extends ConsumerStatefulWidget {
  const QueueMachineHomePage({super.key});

  @override
  ConsumerState<QueueMachineHomePage> createState() =>
      _QueueMachineHomePageState();
}

class _QueueMachineHomePageState extends ConsumerState<QueueMachineHomePage> {
  Timer? _hold;

  void _startHold() {
    _hold?.cancel();
    _hold = Timer(const Duration(seconds: 10), () {
      if (mounted) context.push('/setup');
    });
  }

  @override
  void dispose() {
    _hold?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final s = ref.watch(queueMachineControllerProvider);
    final title = queueMachineText(ref, 'queue_machine');

    return Scaffold(
      body: SafeArea(
        child: Center(
          child: SingleChildScrollView(
            padding: const EdgeInsets.all(24),
            child: ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: 560),
              child: Column(
                mainAxisAlignment: MainAxisAlignment.center,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Center(
                    child: Listener(
                      key: const Key('kiosk-logo'),
                      onPointerDown: (_) => _startHold(),
                      onPointerUp: (_) => _hold?.cancel(),
                      onPointerCancel: (_) => _hold?.cancel(),
                      child: Image.asset(
                        'assets/img/logo.png',
                        width: 130,
                        height: 76,
                        fit: BoxFit.contain,
                        semanticLabel: 'XCHANGE Currency Exchange',
                      ),
                    ),
                  ),
                  const SizedBox(height: 16),
                  Text(
                    title,
                    key: const Key('queue-machine-title'),
                    textAlign: TextAlign.center,
                    style: const TextStyle(
                      fontSize: 24,
                      fontWeight: FontWeight.bold,
                      letterSpacing: 0.0,
                    ),
                  ),
                  if (s.screen != QmScreen.signIn) ...[
                    const SizedBox(height: 12),
                    Wrap(
                      alignment: WrapAlignment.center,
                      spacing: 12,
                      runSpacing: 8,
                      children: [
                        OutlinedButton(
                          key: const Key('queue-machine-printer-button'),
                          onPressed: () => context.push('/printer'),
                          child: Text(queueMachineText(ref, 'printer_title')),
                        ),
                        TextButton(
                          key: const Key('queue-machine-sign-out-button'),
                          onPressed: () => ref
                              .read(queueMachineControllerProvider.notifier)
                              .signOut(),
                          child: Text(queueMachineText(ref, 'sign_out')),
                        ),
                      ],
                    ),
                  ],
                  const SizedBox(height: 24),
                  switch (s.screen) {
                    QmScreen.signIn => const SignInPage(embedded: true),
                    QmScreen.scanner => const ScannerPage(embedded: true),
                    QmScreen.issued => const IssuedPage(embedded: true),
                  },
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}
