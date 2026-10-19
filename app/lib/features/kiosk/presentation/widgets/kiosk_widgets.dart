import 'dart:ui' show FontFeature;

import 'package:flutter/material.dart';

import '../../../../core/platform/display_names.dart';
import '../../../../l10n/kiosk_l10n.dart';
import '../../../../shared/theme/colors.dart';
import '../../domain/entities/kiosk_models.dart';

const _progressFinished = Color(0xFF18794E);
const _progressPendingBorder = Color(0xFFDDDDDD);
const _progressPendingText = Color(0xFF666666);

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

String currencyLabel(DisplayNames names, String code) =>
    '${names.currencySymbol(code)} $code';

String currencyName(DisplayNames names, KioskTexts l10n, CurrencyRate r) =>
    names.currencyName(l10n.language.lang, r.code) ??
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

class _QuoteRow extends StatelessWidget {
  const _QuoteRow(
    this.label,
    this.value, {
    this.ltr = true,
    this.isLast = false,
  });

  final String label, value;
  final bool ltr;
  final bool isLast;

  @override
  Widget build(BuildContext context) {
    final color = isLast ? KioskColors.brand : KioskColors.text;
    final double fontSize = isLast ? 24 : 18;
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 10),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Expanded(
            child: Text(
              label,
              style: TextStyle(
                fontSize: fontSize,
                fontWeight: FontWeight.w400,
                color: color,
              ),
            ),
          ),
          const SizedBox(width: 20),
          Flexible(
            child: ltr
                ? Ltr(
                    value,
                    style: TextStyle(
                      fontSize: fontSize,
                      fontWeight: FontWeight.w700,
                      color: color,
                    ),
                  )
                : Text(
                    value,
                    style: TextStyle(
                      fontSize: fontSize,
                      fontWeight: FontWeight.w700,
                      color: color,
                    ),
                  ),
          ),
        ],
      ),
    );
  }
}

class _IdentityRow extends StatelessWidget {
  const _IdentityRow(
    this.label,
    this.value, {
    this.ltr = true,
    this.isStatus = false,
  });

  final String label, value;
  final bool ltr;
  final bool isStatus;

  @override
  Widget build(BuildContext context) {
    final valueColor = isStatus ? KioskColors.brand : KioskColors.text;
    final valueStyle = TextStyle(
      fontSize: 13,
      fontWeight: FontWeight.w700,
      color: valueColor,
    );

    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 8),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Expanded(
            flex: 1,
            child: Text(
              label,
              style: const TextStyle(
                fontSize: 13,
                fontWeight: FontWeight.w400,
                color: KioskColors.text,
              ),
            ),
          ),
          const SizedBox(width: 14),
          Expanded(
            flex: 2,
            child: DefaultTextStyle.merge(
              textAlign: TextAlign.right,
              child: ltr
                  ? Ltr(value, style: valueStyle)
                  : Text(value, style: valueStyle, textAlign: TextAlign.right),
            ),
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
  Widget build(BuildContext context) => Container(
    width: double.infinity,
    padding: const EdgeInsets.symmetric(vertical: 16, horizontal: 20),
    decoration: BoxDecoration(
      color: const Color(0xFFF8F7F9),
      borderRadius: BorderRadius.circular(12),
    ),
    child: Column(
      children: [
        _IdentityRow(l10n.text('orderNo'), identity.orderNo),
        _IdentityRow(
          l10n.text('transactionNo'),
          identity.transactionNo ?? l10n.text('notCreated'),
        ),
        _IdentityRow(
          l10n.text('status'),
          statusName(l10n, identity),
          ltr: false,
          isStatus: true,
        ),
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
    return Container(
      width: double.infinity,
      padding: const EdgeInsets.symmetric(vertical: 18, horizontal: 22),
      decoration: BoxDecoration(
        color: const Color(0xFFFBFAFB),
        borderRadius: BorderRadius.circular(14),
        border: Border.all(color: KioskColors.line),
      ),
      child: Column(
        children: [
          _QuoteRow(l10n.text('youGive'), give),
          const Divider(height: 1, thickness: 1, color: Color(0xFFEEEEEE)),
          _QuoteRow(
            l10n.text('payment'),
            paymentName(l10n, quote.paymentMethod),
            ltr: false,
          ),
          const Divider(height: 1, thickness: 1, color: Color(0xFFEEEEEE)),
          _QuoteRow(
            l10n.text('rate'),
            '1 ${quote.currency} = ${_formatRate(quote.rate)} THB',
          ),
          const Divider(height: 1, thickness: 1, color: Color(0xFFEEEEEE)),
          _QuoteRow(l10n.text('youReceive'), receive, isLast: true),
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
  Widget build(BuildContext context) => Opacity(
    opacity: onPressed == null ? 0.5 : 1.0,
    child: FilledButton(
      onPressed: onPressed,
      style: FilledButton.styleFrom(
        backgroundColor: KioskColors.brand,
        foregroundColor: Colors.white,
        disabledBackgroundColor: KioskColors.brand,
        disabledForegroundColor: Colors.white,
        minimumSize: const Size.fromHeight(68),
        padding: const EdgeInsets.all(16),
        textStyle: TextStyle(
          fontFamily: Theme.of(context).textTheme.titleLarge?.fontFamily,
          fontFamilyFallback: Theme.of(context)
              .textTheme
              .titleLarge
              ?.fontFamilyFallback,
          fontSize: 24,
          fontWeight: FontWeight.w700,
        ),
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(14)),
      ),
      child: Text(label, textAlign: TextAlign.center),
    ),
  );
}

class SecondaryButton extends StatelessWidget {
  const SecondaryButton({super.key, required this.label, this.onPressed});
  final String label;
  final VoidCallback? onPressed;

  @override
  Widget build(BuildContext context) => Opacity(
    opacity: onPressed == null ? 0.5 : 1.0,
    child: OutlinedButton(
      onPressed: onPressed,
      style: OutlinedButton.styleFrom(
        backgroundColor: Colors.white,
        foregroundColor: KioskColors.text,
        disabledBackgroundColor: Colors.white,
        disabledForegroundColor: KioskColors.text,
        minimumSize: const Size.fromHeight(68),
        padding: const EdgeInsets.all(16),
        textStyle: TextStyle(
          fontFamily: Theme.of(context).textTheme.titleLarge?.fontFamily,
          fontFamilyFallback: Theme.of(context)
              .textTheme
              .titleLarge
              ?.fontFamilyFallback,
          fontSize: 22,
          fontWeight: FontWeight.w700,
        ),
        side: const BorderSide(color: KioskColors.line, width: 2),
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(14)),
      ),
      child: Text(label, textAlign: TextAlign.center),
    ),
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
            for (var i = 0; i < labels.length; i++) ...[
              if (i > 0) const SizedBox(width: 12),
              Expanded(
                child: Container(
                  padding: const EdgeInsets.symmetric(
                    vertical: 18,
                    horizontal: 8,
                  ),
                  decoration: BoxDecoration(
                    color: i == stage ? KioskColors.brand : Colors.white,
                    borderRadius: BorderRadius.circular(14),
                    border: Border.all(
                      color: i == stage
                          ? KioskColors.brand
                          : i < stage
                          ? _progressFinished
                          : _progressPendingBorder,
                      width: 2,
                    ),
                  ),
                  child: Text(
                    '${i < stage ? '✓ ' : ''}${l10n.text(labels[i])}',
                    textAlign: TextAlign.center,
                    style: TextStyle(
                      fontSize: 18,
                      fontWeight: FontWeight.w400,
                      color: i == stage
                          ? Colors.white
                          : i < stage
                          ? _progressFinished
                          : _progressPendingText,
                    ),
                  ),
                ),
              ),
            ],
          ],
        ),
        const SizedBox(height: 16),
        Text(
          l10n.text(phaseKey(status)),
          textAlign: TextAlign.center,
          style: const TextStyle(
            fontSize: 18,
            fontWeight: FontWeight.w400,
            color: KioskColors.text,
          ),
        ),
      ],
    );
  }
}

class QueueNumber extends StatelessWidget {
  const QueueNumber(this.queue, {super.key});
  final String queue;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.symmetric(vertical: 14),
    child: Ltr(
      queue,
      style: const TextStyle(
        fontSize: 44,
        fontWeight: FontWeight.w700,
        color: KioskColors.brand,
        letterSpacing: 2,
        fontFeatures: [FontFeature.tabularFigures()],
      ),
    ),
  );
}

String _formatRate(double rate) {
  final s = rate.toString();
  final dot = s.indexOf('.');
  if (dot < 0) return '$s.0000';
  final decimals = s.length - dot - 1;
  return decimals < 4 ? s.padRight(dot + 5, '0') : s;
}
