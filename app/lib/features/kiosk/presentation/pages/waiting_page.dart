import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../../core/platform/device_actions.dart';
import '../../../../shared/theme/colors.dart';
import '../../app/kiosk_providers.dart';
import '../widgets/kiosk_widgets.dart';

/// Screen 3 — the request is with staff: queue number and where it stands.
class WaitingPage extends ConsumerWidget {
  const WaitingPage({super.key});

  static const telegramUrl = 'https://t.me/XCHANGEXTK';

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final s = ref.watch(kioskControllerProvider);
    final l = ref.watch(kioskTextsProvider);
    final id = s.identity!;
    return _DoneCard(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Center(
            child: Container(
              width: 64,
              height: 64,
              decoration: const BoxDecoration(
                color: Color(0xFFFFF0F5),
                shape: BoxShape.circle,
              ),
              child: const Icon(
                Icons.check,
                color: KioskColors.brand,
                size: 40,
              ),
            ),
          ),
          const SizedBox(height: 12),
          Text(
            l.text('sentTitle'),
            textAlign: TextAlign.center,
            style: const TextStyle(fontSize: 25, fontWeight: FontWeight.w700),
          ),
          const SizedBox(height: 8),
          Center(child: QueueNumber(id.queue)),
          const SizedBox(height: 16),
          IdentityBlock(l10n: l, identity: id),
          const SizedBox(height: 20),
          Text(
            l.text('bring'),
            textAlign: TextAlign.center,
            style: const TextStyle(fontSize: 25, fontWeight: FontWeight.w700),
          ),
          const SizedBox(height: 6),
          Text(
            l.text('requestOnly'),
            textAlign: TextAlign.center,
            style: const TextStyle(fontSize: 17, color: KioskColors.muted),
          ),
          const SizedBox(height: 20),
          OrderProgress(l10n: l, status: id.status),
          const SizedBox(height: 24),
          Text(
            l.text('summaryTitle'),
            textAlign: TextAlign.center,
            style: const TextStyle(fontSize: 25, fontWeight: FontWeight.w700),
          ),
          const SizedBox(height: 10),
          QuoteSummary(l10n: l, quote: s.quote!),
          const SizedBox(height: 8),
          Text(
            l.text('summaryNote'),
            textAlign: TextAlign.center,
            style: const TextStyle(fontSize: 17, color: KioskColors.muted),
          ),
          const SizedBox(height: 28),
          KioskCard(
            color: const Color(0xFFFAF9FB),
            child: Row(
              children: [
                Image.asset(
                  'assets/img/telegram_qr.png',
                  width: 120,
                  height: 120,
                  filterQuality: FilterQuality.none,
                  fit: BoxFit.contain,
                  semanticLabel: 'QR code linking to $telegramUrl',
                ),
                const SizedBox(width: 24),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        l.text('usdtTitle'),
                        style: const TextStyle(
                          fontSize: 20,
                          fontWeight: FontWeight.w700,
                        ),
                      ),
                      Text(
                        l.text('usdtScan'),
                        style: const TextStyle(fontSize: 18),
                      ),
                      TextButton(
                        onPressed: () async {
                          // Read before the await: the page may be gone when
                          // it fails.
                          final actions = ref.read(deviceActionsProvider);
                          final kiosk = ref.read(
                            kioskControllerProvider.notifier,
                          );
                          try {
                            await actions.openExternal(telegramUrl);
                          } on DeviceActionException {
                            kiosk.reportError('errGeneric');
                          }
                        },
                        style: TextButton.styleFrom(
                          padding: EdgeInsets.zero,
                          foregroundColor: KioskColors.brand,
                          textStyle: const TextStyle(
                            fontSize: 18,
                            fontWeight: FontWeight.w700,
                          ),
                        ),
                        child: const Text(
                          '@XCHANGEXTK',
                          style: TextStyle(
                            fontSize: 18,
                            fontWeight: FontWeight.w700,
                          ),
                        ),
                      ),
                      Text(
                        l.text('usdtNote'),
                        style: const TextStyle(fontSize: 18),
                      ),
                    ],
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class _DoneCard extends StatelessWidget {
  const _DoneCard({required this.child});
  final Widget child;

  @override
  Widget build(BuildContext context) => Center(
    child: ConstrainedBox(
      constraints: const BoxConstraints(maxWidth: 880),
      child: Container(
        width: double.infinity,
        padding: const EdgeInsets.all(30),
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
