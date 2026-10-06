import 'package:flutter/material.dart';

import '../../../../core/platform/browser.dart' as platform;
import '../../../../l10n/kiosk_l10n.dart';
import '../../../../shared/theme/colors.dart';
import '../../domain/entities/kiosk_models.dart';

/// `1234567.5` → `1,234,567.50`.
String formatMoney(num n) {
  final fixed = n.abs().toStringAsFixed(2);
  final dot = fixed.indexOf('.');
  final whole = fixed.substring(0, dot);
  final grouped = StringBuffer();
  for (var i = 0; i < whole.length; i++) {
    if (i > 0 && (whole.length - i) % 3 == 0) grouped.write(',');
    grouped.write(whole[i]);
  }
  return '${n < 0 ? '-' : ''}$grouped${fixed.substring(dot)}';
}

String currencyLabel(String code) => '${platform.currencySymbol(code)} $code';

String currencyName(KioskTexts l10n, CurrencyRate r) =>
    platform.currencyDisplayName(l10n.language.lang, r.code) ??
    (l10n.code == 'th' && r.name.isNotEmpty
        ? r.name
        : r.nameEn.isNotEmpty
        ? r.nameEn
        : r.code);

String paymentName(KioskTexts l10n, String method) => switch (method) {
  'Credit Card' => l10n.text('creditCard'),
  'Cash' => l10n.text('cash'),
  'Bank Transfer' => l10n.text('transfer'),
  _ => method,
};

String statusName(KioskTexts l10n, RequestIdentity id) {
  final key = 'st${id.rawStatus}';
  return l10n.has(key) ? l10n.text(key) : id.rawStatus;
}

/// Amounts and codes read left to right even inside right-to-left text.
class Ltr extends StatelessWidget {
  const Ltr(this.text, {super.key, this.style});
  final String text;
  final TextStyle? style;

  @override
  Widget build(BuildContext context) => Directionality(
    textDirection: TextDirection.ltr,
    child: Text(text, style: style),
  );
}

class KioskCard extends StatelessWidget {
  const KioskCard({super.key, required this.child, this.color = Colors.white});
  final Widget child;
  final Color color;

  @override
  Widget build(BuildContext context) => Container(
    width: double.infinity,
    padding: const EdgeInsets.all(24),
    decoration: BoxDecoration(
      color: color,
      borderRadius: BorderRadius.circular(18),
      border: Border.all(color: KioskColors.line),
    ),
    child: child,
  );
}

class _Row extends StatelessWidget {
  const _Row(this.label, this.value, {this.ltr = true});
  final String label, value;
  final bool ltr;

  @override
  Widget build(BuildContext context) {
    const style = TextStyle(fontSize: 20, fontWeight: FontWeight.w700);
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 10),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Expanded(
            child: Text(
              label,
              style: const TextStyle(fontSize: 18, color: KioskColors.muted),
            ),
          ),
          const SizedBox(width: 16),
          Flexible(
            child: ltr ? Ltr(value, style: style) : Text(value, style: style),
          ),
        ],
      ),
    );
  }
}

/// Order number, transaction number and status.
class IdentityBlock extends StatelessWidget {
  const IdentityBlock({super.key, required this.l10n, required this.identity});
  final KioskTexts l10n;
  final RequestIdentity identity;

  @override
  Widget build(BuildContext context) => KioskCard(
    color: KioskColors.soft,
    child: Column(
      children: [
        _Row(l10n.text('orderNo'), identity.orderNo),
        _Row(
          l10n.text('transactionNo'),
          identity.transactionNo ?? l10n.text('notCreated'),
        ),
        _Row(l10n.text('status'), statusName(l10n, identity), ltr: false),
      ],
    ),
  );
}

/// What the customer gives, how, at what rate, and what they receive — all
/// figures as the server quoted them.
class QuoteSummary extends StatelessWidget {
  const QuoteSummary({super.key, required this.l10n, required this.quote});
  final KioskTexts l10n;
  final QuoteModel quote;

  @override
  Widget build(BuildContext context) {
    final buy = quote.type == ExchangeType.buy;
    final give = buy
        ? '${formatMoney(quote.foreign)} ${quote.currency}'
        : '${formatMoney(quote.thb)} THB';
    final receive = buy
        ? '${formatMoney(quote.thb)} THB'
        : '${formatMoney(quote.foreign)} ${quote.currency}';
    return KioskCard(
      child: Column(
        children: [
          _Row(l10n.text('youGive'), give),
          _Row(
            l10n.text('payment'),
            paymentName(l10n, quote.paymentMethod),
            ltr: false,
          ),
          _Row(
            l10n.text('rate'),
            '1 ${quote.currency} = ${quote.rate.toStringAsFixed(4)} THB',
          ),
          _Row(l10n.text('youReceive'), receive),
        ],
      ),
    );
  }
}

class PrimaryButton extends StatelessWidget {
  const PrimaryButton({super.key, required this.label, this.onPressed});
  final String label;
  final VoidCallback? onPressed;

  @override
  Widget build(BuildContext context) => FilledButton(
    onPressed: onPressed,
    style: FilledButton.styleFrom(
      backgroundColor: KioskColors.brand,
      minimumSize: const Size.fromHeight(72),
      textStyle: Theme.of(context).textTheme.titleLarge?.copyWith(
        fontSize: 24,
        fontWeight: FontWeight.w700,
      ),
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(16)),
    ),
    child: Text(label, textAlign: TextAlign.center),
  );
}

class SecondaryButton extends StatelessWidget {
  const SecondaryButton({super.key, required this.label, this.onPressed});
  final String label;
  final VoidCallback? onPressed;

  @override
  Widget build(BuildContext context) => OutlinedButton(
    onPressed: onPressed,
    style: OutlinedButton.styleFrom(
      foregroundColor: KioskColors.ink,
      minimumSize: const Size.fromHeight(72),
      textStyle: Theme.of(context).textTheme.titleLarge?.copyWith(
        fontSize: 22,
        fontWeight: FontWeight.w700,
      ),
      side: const BorderSide(color: KioskColors.line, width: 2),
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(16)),
    ),
    child: Text(label, textAlign: TextAlign.center),
  );
}

/// The three steps staff take the customer through at the counter.
class OrderProgress extends StatelessWidget {
  const OrderProgress({super.key, required this.l10n, required this.status});
  final KioskTexts l10n;
  final KioskStatus status;

  static int stageOf(KioskStatus s) => switch (s) {
    KioskStatus.accepted => 0,
    KioskStatus.idChecked => 1,
    KioskStatus.completed => 2,
    _ => -1,
  };

  static String phaseKey(KioskStatus s) => switch (stageOf(s)) {
    0 => 'phasePassport',
    1 => 'phaseProcessing',
    2 => 'phaseDone',
    _ => 'phaseWaiting',
  };

  @override
  Widget build(BuildContext context) {
    final stage = stageOf(status);
    const labels = ['stepId', 'stepPending', 'stepSuccess'];
    return Column(
      children: [
        Row(
          children: [
            for (var i = 0; i < labels.length; i++)
              Expanded(
                child: Container(
                  margin: const EdgeInsets.symmetric(horizontal: 4),
                  padding: const EdgeInsets.symmetric(vertical: 16),
                  decoration: BoxDecoration(
                    color: i == stage
                        ? KioskColors.brand
                        : i < stage
                        ? KioskColors.soft
                        : Colors.white,
                    borderRadius: BorderRadius.circular(14),
                    border: Border.all(
                      color: i <= stage ? KioskColors.brand : KioskColors.line,
                      width: 2,
                    ),
                  ),
                  child: Text(
                    '${i < stage ? '✓ ' : ''}${l10n.text(labels[i])}',
                    textAlign: TextAlign.center,
                    style: TextStyle(
                      fontSize: 18,
                      fontWeight: FontWeight.w700,
                      color: i == stage ? Colors.white : KioskColors.ink,
                    ),
                  ),
                ),
              ),
          ],
        ),
        const SizedBox(height: 16),
        Text(
          l10n.text(phaseKey(status)),
          textAlign: TextAlign.center,
          style: const TextStyle(fontSize: 22, fontWeight: FontWeight.w600),
        ),
      ],
    );
  }
}

class QueueNumber extends StatelessWidget {
  const QueueNumber(this.queue, {super.key});
  final String queue;

  @override
  Widget build(BuildContext context) => Ltr(
    queue,
    style: const TextStyle(
      fontSize: 64,
      fontWeight: FontWeight.w800,
      color: KioskColors.brand,
      letterSpacing: 2,
    ),
  );
}
