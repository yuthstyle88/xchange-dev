import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../../core/platform/browser.dart' as platform;
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
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const Center(
          child: CircleAvatar(
            radius: 36,
            backgroundColor: KioskColors.brand,
            child: Icon(Icons.check, color: Colors.white, size: 44),
          ),
        ),
        const SizedBox(height: 12),
        Text(
          l.text('sentTitle'),
          textAlign: TextAlign.center,
          style: const TextStyle(fontSize: 30, fontWeight: FontWeight.w800),
        ),
        const SizedBox(height: 8),
        Center(child: QueueNumber(id.queue)),
        const SizedBox(height: 16),
        IdentityBlock(l10n: l, identity: id),
        const SizedBox(height: 20),
        Text(
          l.text('bring'),
          textAlign: TextAlign.center,
          style: const TextStyle(fontSize: 22, fontWeight: FontWeight.w700),
        ),
        const SizedBox(height: 6),
        Text(
          l.text('requestOnly'),
          textAlign: TextAlign.center,
          style: const TextStyle(fontSize: 16, color: KioskColors.muted),
        ),
        const SizedBox(height: 20),
        OrderProgress(l10n: l, status: id.status),
        const SizedBox(height: 24),
        Text(
          l.text('summaryTitle'),
          style: const TextStyle(fontSize: 22, fontWeight: FontWeight.w800),
        ),
        const SizedBox(height: 10),
        QuoteSummary(l10n: l, quote: s.quote!),
        const SizedBox(height: 8),
        Text(
          l.text('summaryNote'),
          style: const TextStyle(fontSize: 16, color: KioskColors.muted),
        ),
        const SizedBox(height: 20),
        KioskCard(
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
              const SizedBox(width: 20),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      l.text('usdtTitle'),
                      style: const TextStyle(
                        fontSize: 20,
                        fontWeight: FontWeight.w800,
                      ),
                    ),
                    Text(l.text('usdtScan')),
                    TextButton(
                      onPressed: () => platform.openExternal(telegramUrl),
                      style: TextButton.styleFrom(
                        padding: EdgeInsets.zero,
                        foregroundColor: KioskColors.brand,
                      ),
                      child: const Text('@XCHANGEXTK'),
                    ),
                    Text(
                      l.text('usdtNote'),
                      style: const TextStyle(color: KioskColors.muted),
                    ),
                  ],
                ),
              ),
            ],
          ),
        ),
      ],
    );
  }
}
