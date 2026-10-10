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
    return Center(
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 880),
        child: Container(
          width: double.infinity,
          padding: const EdgeInsets.all(30),
          decoration: BoxDecoration(
            color: Colors.white,
            border: Border.all(color: KioskColors.line),
            borderRadius: BorderRadius.circular(20),
            boxShadow: const [
              BoxShadow(
                color: Color(0x0628202B),
                offset: Offset(0, 12),
                blurRadius: 40,
              ),
            ],
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Text(
                l.text('reviewTitle'),
                style: const TextStyle(
                  fontSize: 25,
                  fontWeight: FontWeight.w700,
                ),
              ),
              const SizedBox(height: 16),
              IdentityBlock(l10n: l, identity: s.identity!),
              const SizedBox(height: 16),
              QuoteSummary(l10n: l, quote: quote),
              const SizedBox(height: 12),
              Text(
                l.text('feeNote'),
                style: const TextStyle(fontSize: 17, color: KioskColors.muted),
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
                  const SizedBox(width: 14),
                  Expanded(
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
          ),
        ),
      ),
    );
  }
}
