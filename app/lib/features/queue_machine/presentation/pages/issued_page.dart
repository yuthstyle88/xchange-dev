import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../../shared/theme/colors.dart';
import '../controllers/queue_machine_controller.dart';
import 'queue_machine_home_page.dart';

class IssuedPage extends ConsumerStatefulWidget {
  const IssuedPage({super.key, this.embedded = false});

  final bool embedded;

  @override
  ConsumerState<IssuedPage> createState() => _IssuedPageState();
}

class _IssuedPageState extends ConsumerState<IssuedPage> {
  final TextEditingController _rescanInput = TextEditingController();

  @override
  void dispose() {
    _rescanInput.dispose();
    super.dispose();
  }

  void _submitRescan(String raw) {
    ref.read(queueMachineControllerProvider.notifier).onScan(raw);
    _rescanInput.clear();
  }

  @override
  Widget build(BuildContext context) {
    final s = ref.watch(queueMachineControllerProvider);
    final statusKey = switch (s.print) {
      PrintStatus.printed => 'issued_printed',
      PrintStatus.failed => 'issued_print_failed',
      PrintStatus.noPrinter => 'issued_noPrinter',
      PrintStatus.none => null,
    };
    final statusText = switch (s.print) {
      PrintStatus.printed => queueMachineText(ref, 'issued_printed'),
      PrintStatus.failed => queueMachineText(ref, 'issued_print_failed'),
      PrintStatus.noPrinter => queueMachineText(ref, 'issued_no_printer'),
      PrintStatus.none => null,
    };

    final content = Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text(
          queueMachineText(ref, 'issued_title'),
          key: const Key('issued-title'),
          textAlign: TextAlign.center,
          style: const TextStyle(fontSize: 22, fontWeight: FontWeight.w700),
        ),
        const SizedBox(height: 16),
        Text(
          s.displayNumber ?? '',
          key: const Key('issued-number'),
          textAlign: TextAlign.center,
          style: const TextStyle(
            fontSize: 72,
            fontWeight: FontWeight.w800,
            color: KioskColors.brand,
          ),
        ),
        if (statusKey != null && statusText != null) ...[
          const SizedBox(height: 16),
          Text(
            statusText,
            key: const Key('issued-print-status'),
            textAlign: TextAlign.center,
            style: TextStyle(
              fontSize: 20,
              fontWeight: FontWeight.w600,
              color: s.print == PrintStatus.failed
                  ? Colors.red
                  : KioskColors.text,
            ),
          ),
        ],
        const SizedBox(height: 24),
        TextField(
          key: const Key('issued-scan-input'),
          controller: _rescanInput,
          autocorrect: false,
          onSubmitted: _submitRescan,
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
