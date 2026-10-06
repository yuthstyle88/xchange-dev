import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../../shared/theme/colors.dart';
import '../../app/kiosk_providers.dart';
import '../widgets/kiosk_widgets.dart';

/// Screen 2 — the server's quote, with the review countdown before confirm.
class QuotePage extends ConsumerWidget {
  const QuotePage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final s = ref.watch(kioskControllerProvider);
    final c = ref.read(kioskControllerProvider.notifier);
    final l = ref.watch(kioskTextsProvider);
    final quote = s.quote!;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text(
          l.text('reviewTitle'),
          style: const TextStyle(fontSize: 30, fontWeight: FontWeight.w800),
        ),
        const SizedBox(height: 16),
        IdentityBlock(l10n: l, identity: s.identity!),
        const SizedBox(height: 16),
        QuoteSummary(l10n: l, quote: quote),
        const SizedBox(height: 12),
        Text(
          l.text('feeNote'),
          style: const TextStyle(fontSize: 16, color: KioskColors.muted),
        ),
        const SizedBox(height: 24),
        Row(
          children: [
            Expanded(
              child: SecondaryButton(
                key: const Key('back'),
                label: l.text('back'),
                onPressed: s.busy ? null : c.backToSelect,
              ),
            ),
            const SizedBox(width: 16),
            Expanded(
              flex: 2,
              child: PrimaryButton(
                key: const Key('confirm'),
                label: s.secondsLeft > 0
                    ? l.text('wait', {'n': s.secondsLeft})
                    : l.text('confirm'),
                onPressed: s.canConfirm ? c.confirm : null,
              ),
            ),
          ],
        ),
      ],
    );
  }
}
