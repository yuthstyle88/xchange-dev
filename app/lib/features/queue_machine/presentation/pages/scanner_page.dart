import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:mobile_scanner/mobile_scanner.dart';

import '../controllers/queue_machine_controller.dart';
import 'queue_machine_home_page.dart';

class ScannerPage extends ConsumerStatefulWidget {
  const ScannerPage({super.key, this.embedded = false});

  final bool embedded;

  @override
  ConsumerState<ScannerPage> createState() => _ScannerPageState();
}

class _ScannerPageState extends ConsumerState<ScannerPage> {
  final TextEditingController _qrInput = TextEditingController();

  @override
  void dispose() {
    _qrInput.dispose();
    super.dispose();
  }

  void _submit(String raw) {
    ref.read(queueMachineControllerProvider.notifier).onScan(raw);
    _qrInput.clear();
  }

  @override
  Widget build(BuildContext context) {
    final s = ref.watch(queueMachineControllerProvider);
    final errorKey = switch (s.error) {
      null => null,
      'invalidQr' => 'scanner_error_invalid_qr',
      _ => 'scanner_error_not_found',
    };

    final content = Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text(
          queueMachineText(ref, 'scanner_title'),
          key: const Key('scanner-title'),
          style: const TextStyle(fontSize: 22, fontWeight: FontWeight.w700),
        ),
        const SizedBox(height: 16),
        SizedBox(
          height: 220,
          child: ClipRRect(
            borderRadius: BorderRadius.circular(12),
            child: MobileScanner(
              key: const Key('scanner-camera'),
              placeholderBuilder: (context) =>
                  const ColoredBox(color: Colors.black12),
              errorBuilder: (context, error) =>
                  const ColoredBox(color: Colors.black12),
              onDetect: (capture) {
                for (final barcode in capture.barcodes) {
                  final raw = barcode.rawValue;
                  if (raw != null && raw.isNotEmpty) {
                    _submit(raw);
                    break;
                  }
                }
              },
            ),
          ),
        ),
        const SizedBox(height: 16),
        Text(queueMachineText(ref, 'scanner_input_label')),
        const SizedBox(height: 6),
        TextField(
          key: const Key('scanner-input'),
          controller: _qrInput,
          autocorrect: false,
          onSubmitted: _submit,
        ),
        if (errorKey != null) ...[
          const SizedBox(height: 12),
          Text(
            queueMachineText(ref, errorKey),
            key: const Key('scanner-error'),
            style: const TextStyle(fontSize: 16, color: Colors.red),
          ),
        ],
        const SizedBox(height: 16),
        FilledButton(
          key: const Key('scanner-submit'),
          onPressed: () => _submit(_qrInput.text),
          child: Text(queueMachineText(ref, 'scanner_submit')),
        ),
      ],
    );

    if (widget.embedded) return content;

    return Scaffold(
      body: SafeArea(
        child: Center(
          child: SingleChildScrollView(
            padding: const EdgeInsets.all(24),
            child: ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: 480),
              child: content,
            ),
          ),
        ),
      ),
    );
  }
}
