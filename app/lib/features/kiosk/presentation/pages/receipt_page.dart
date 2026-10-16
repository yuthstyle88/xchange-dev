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
class ReceiptPage extends ConsumerStatefulWidget {
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
  ConsumerState<ReceiptPage> createState() => _ReceiptPageState();
}

class _ReceiptPageState extends ConsumerState<ReceiptPage> {
  bool _saving = false;

  Future<void> _saveReceipt(String image, RequestIdentity id) async {
    if (_saving) return;
    setState(() => _saving = true);
    // Read before the await: the page may be gone when it fails.
    final actions = ref.read(deviceActionsProvider);
    final kiosk = ref.read(kioskControllerProvider.notifier);
    try {
      await actions.saveImage(image, ReceiptPage.filename(id));
    } on DeviceActionException {
      kiosk.reportError('errGeneric');
    } finally {
      if (mounted) {
        setState(() => _saving = false);
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final width = MediaQuery.sizeOf(context).width;
    final narrow = width <= 760;
    final s = ref.watch(kioskControllerProvider);
    final l = ref.watch(kioskTextsProvider);
    final id = s.identity!;
    final image = s.receiptImage!;
    final bytes = ReceiptPage.decode(image);
    return _DoneCard(
      padding: EdgeInsets.all(narrow ? 20 : 30),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(
            l.text('receiptTitle'),
            textAlign: TextAlign.center,
            style: const TextStyle(fontSize: 25, fontWeight: FontWeight.w700),
          ),
          const SizedBox(height: 6),
          Text(
            l.text(
              id.status == KioskStatus.completed
                  ? 'stCOMPLETED'
                  : 'stID_CHECKED',
            ),
            textAlign: TextAlign.center,
            style: const TextStyle(fontSize: 18),
          ),
          const SizedBox(height: 8),
          Center(child: QueueNumber(id.queue)),
          const SizedBox(height: 16),
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
                onPressed: _saving ? null : () => _saveReceipt(image, id),
              ),
            ),
          ),
          const SizedBox(height: 20),
          OrderProgress(l10n: l, status: id.status),
          const SizedBox(height: 16),
          IdentityBlock(l10n: l, identity: id),
        ],
      ),
    );
  }
}

class _DoneCard extends StatelessWidget {
  const _DoneCard({
    required this.child,
    this.padding = const EdgeInsets.all(30),
  });
  final Widget child;
  final EdgeInsets padding;

  @override
  Widget build(BuildContext context) => Center(
    child: ConstrainedBox(
      constraints: const BoxConstraints(maxWidth: 880),
      child: Container(
        width: double.infinity,
        padding: padding,
        decoration: BoxDecoration(
          color: Colors.white,
          borderRadius: BorderRadius.circular(20),
          border: Border.all(color: KioskColors.line),
          boxShadow: const [
            BoxShadow(
              color: Color(0x0628202B),
              offset: Offset(0, 12),
              blurRadius: 40,
            ),
          ],
        ),
        child: child,
      ),
    ),
  );
}
