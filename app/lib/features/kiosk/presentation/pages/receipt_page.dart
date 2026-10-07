import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../../core/platform/device_actions.dart';
import '../../../../shared/theme/colors.dart';
import '../../app/kiosk_providers.dart';
import '../../domain/entities/kiosk_models.dart';
import '../widgets/kiosk_widgets.dart';

/// Screen 4 — the receipt image the staff member published to this screen.
class ReceiptPage extends ConsumerWidget {
  const ReceiptPage({super.key});

  /// The bytes of a `data:image/png;base64,…` URL, or null if it is not one.
  static Uint8List? decode(String dataUrl) {
    const prefix = 'data:image/png;base64,';
    if (!dataUrl.startsWith(prefix)) return null;
    try {
      return base64Decode(dataUrl.substring(prefix.length));
    } on FormatException {
      return null;
    }
  }

  static String filename(RequestIdentity id) =>
      '${id.transactionNo ?? '${id.queue}_PENDING'}_RECEIPT.png';

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final s = ref.watch(kioskControllerProvider);
    final l = ref.watch(kioskTextsProvider);
    final id = s.identity!;
    final image = s.receiptImage!;
    final bytes = decode(image);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text(
          l.text('receiptTitle'),
          textAlign: TextAlign.center,
          style: const TextStyle(fontSize: 30, fontWeight: FontWeight.w800),
        ),
        const SizedBox(height: 6),
        Text(
          l.text(
            id.status == KioskStatus.completed ? 'stCOMPLETED' : 'stID_CHECKED',
          ),
          textAlign: TextAlign.center,
          style: const TextStyle(fontSize: 20, color: KioskColors.muted),
        ),
        const SizedBox(height: 8),
        Center(child: QueueNumber(id.queue)),
        const SizedBox(height: 12),
        if (bytes != null)
          Center(
            child: ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: 400),
              child: Image.memory(
                bytes,
                gaplessPlayback: true,
                semanticLabel: 'Exchange receipt',
              ),
            ),
          ),
        const SizedBox(height: 16),
        Center(
          child: ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 400),
            child: PrimaryButton(
              key: const Key('save-receipt'),
              label: l.text('saveImage'),
              onPressed: () async {
                // Read before the await: the page may be gone when it fails.
                final actions = ref.read(deviceActionsProvider);
                final kiosk = ref.read(kioskControllerProvider.notifier);
                try {
                  await actions.saveImage(image, filename(id));
                } on DeviceActionException {
                  kiosk.reportError('errGeneric');
                }
              },
            ),
          ),
        ),
        const SizedBox(height: 20),
        OrderProgress(l10n: l, status: id.status),
        const SizedBox(height: 16),
        IdentityBlock(l10n: l, identity: id),
      ],
    );
  }
}
