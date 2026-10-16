import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../../core/platform/display_names.dart';
import '../../../../l10n/kiosk_l10n.dart';
import '../../../../shared/theme/colors.dart';
import '../../app/kiosk_providers.dart';
import '../../domain/entities/kiosk_models.dart';
import '../controllers/kiosk_controller.dart';
import '../widgets/kiosk_widgets.dart';

/// Screen 1 — choose buy or sell, the currency and the amount; see the rate.
class SelectPage extends ConsumerStatefulWidget {
  const SelectPage({super.key});

  @override
  ConsumerState<SelectPage> createState() => _SelectPageState();
}

class _SelectPageState extends ConsumerState<SelectPage> {
  final _amount = TextEditingController();
  final _search = TextEditingController();

  KioskState get s => ref.watch(kioskControllerProvider);
  KioskController get c => ref.read(kioskControllerProvider.notifier);
  KioskTexts get l => ref.watch(kioskTextsProvider);
  DisplayNames get names => ref.watch(displayNamesProvider);

  @override
  void initState() {
    super.initState();
    _amount.text = ref.read(kioskControllerProvider).amountText;
  }

  @override
  void dispose() {
    _amount.dispose();
    _search.dispose();
    super.dispose();
  }

  List<CurrencyRate> _matches() {
    final q = _search.text.trim().toLowerCase();
    final lang = l.language.lang;
    final dNames = names;
    return [
      for (final r in s.model?.rates ?? const <CurrencyRate>[])
        if (q.isEmpty ||
            [
              r.code,
              r.name,
              r.nameEn,
              currencyName(dNames, l, r),
              for (final k in r.countries) ...[
                k.code,
                k.en,
                k.th,
                dNames.regionName(lang, k.code) ?? '',
              ],
            ].join(' ').toLowerCase().contains(q))
          r,
    ];
  }

  @override
  Widget build(BuildContext context) {
    // A reset clears the amount in the state; the text field follows it.
    ref.listen(kioskControllerProvider.select((x) => x.amountText), (_, now) {
      if (now.isEmpty && _amount.text.isNotEmpty) {
        _amount.clear();
        _search.clear();
      }
    });
    final buy = s.type == ExchangeType.buy;
    final wide = MediaQuery.sizeOf(context).width >= 900;
    final direction = Row(
      children: [
        Expanded(
          child: _DirectionButton(
            title: l.text('buy'),
            hint: l.text('buyHint'),
            selected: buy,
            onTap: () => c.setType(ExchangeType.buy),
          ),
        ),
        const SizedBox(width: 16),
        Expanded(
          child: _DirectionButton(
            title: l.text('sell'),
            hint: l.text('sellHint'),
            selected: !buy,
            onTap: () => c.setType(ExchangeType.sell),
          ),
        ),
      ],
    );
    final panes = [_currencyPane(), _amountPane()];
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        direction,
        const SizedBox(height: 20),
        if (wide)
          Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Expanded(child: panes[0]),
              const SizedBox(width: 20),
              Expanded(child: panes[1]),
            ],
          )
        else ...[
          panes[0],
          const SizedBox(height: 20),
          panes[1],
        ],
      ],
    );
  }

  Widget _currencyPane() {
    final matches = _matches();
    return KioskCard(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(
            l.text('currencyLabel'),
            style: const TextStyle(fontSize: 20, fontWeight: FontWeight.w700),
          ),
          const SizedBox(height: 12),
          TextField(
            controller: _search,
            onChanged: (_) => setState(() {}),
            style: const TextStyle(fontSize: 20),
            decoration: InputDecoration(
              hintText: l.text('searchPlaceholder'),
              prefixIcon: const Icon(Icons.search),
            ),
          ),
          const SizedBox(height: 16),
          if (s.model != null && matches.isEmpty)
            Text(
              l.text('noCurrencies'),
              style: const TextStyle(fontSize: 18, color: KioskColors.muted),
            )
          else
            Wrap(
              spacing: 12,
              runSpacing: 12,
              children: [
                for (final r in matches)
                  _CurrencyChip(
                    code: r.code,
                    label: currencyLabel(names, r.code),
                    name: currencyName(names, l, r),
                    selected: r.code == s.currency,
                    onTap: () => c.setCurrency(r.code),
                  ),
              ],
            ),
        ],
      ),
    );
  }

  Widget _amountPane() {
    final buy = s.type == ExchangeType.buy;
    final code = s.currency ?? '';
    final rate = s.selectedRate?.rateFor(s.type) ?? 0;
    final give = buy ? code : 'THB';
    final receive = buy ? 'THB' : code;
    final estimate = s.estimate;
    final receiveText = estimate == null
        ? ''
        : '${names.currencySymbol(receive)} '
              '${formatMoney(estimate.receive)} $receive';
    final freshness = s.model?.freshness;
    return KioskCard(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Container(
            padding: const EdgeInsets.all(16),
            decoration: BoxDecoration(
              color: KioskColors.soft,
              borderRadius: BorderRadius.circular(14),
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  l.text(buy ? 'weBuy' : 'weSell'),
                  style: const TextStyle(
                    fontSize: 16,
                    color: KioskColors.muted,
                  ),
                ),
                const SizedBox(height: 4),
                rate > 0
                    ? Ltr(
                        '1 ${currencyLabel(names, code)} = '
                        '${_formatRate(rate)} ฿ THB',
                        style: const TextStyle(
                          fontSize: 26,
                          fontWeight: FontWeight.w800,
                        ),
                      )
                    : Text(
                        l.text('notAvailable'),
                        style: const TextStyle(
                          fontSize: 26,
                          fontWeight: FontWeight.w800,
                        ),
                      ),
                if (freshness != null) ...[
                  const SizedBox(height: 8),
                  _freshnessBlock(freshness),
                ],
              ],
            ),
          ),
          const SizedBox(height: 20),
          Text(
            l.text('amountLabel', {'code': currencyLabel(names, give)}),
            style: const TextStyle(fontSize: 18, fontWeight: FontWeight.w700),
          ),
          const SizedBox(height: 8),
          Directionality(
            textDirection: TextDirection.ltr,
            child: TextField(
              key: const Key('amount'),
              controller: _amount,
              onChanged: c.setAmount,
              keyboardType: const TextInputType.numberWithOptions(
                decimal: true,
              ),
              inputFormatters: [
                FilteringTextInputFormatter.allow(
                  RegExp(r'^\d{0,8}(\.\d{0,2})?'),
                ),
              ],
              style: const TextStyle(fontSize: 34, fontWeight: FontWeight.w700),
              decoration: const InputDecoration(hintText: '0.00'),
            ),
          ),
          const SizedBox(height: 20),
          Text(
            '${l.text('youReceive')} (${currencyLabel(names, receive)})',
            style: const TextStyle(fontSize: 18, fontWeight: FontWeight.w700),
          ),
          const SizedBox(height: 8),
          Container(
            padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 18),
            decoration: BoxDecoration(
              color: KioskColors.soft,
              borderRadius: BorderRadius.circular(14),
              border: Border.all(color: KioskColors.line),
            ),
            child: Ltr(
              receiveText.isEmpty ? '0.00' : receiveText,
              style: TextStyle(
                fontSize: 34,
                fontWeight: FontWeight.w700,
                color: receiveText.isEmpty
                    ? KioskColors.muted
                    : KioskColors.ink,
              ),
            ),
          ),
          if (estimate != null) ...[
            const SizedBox(height: 8),
            Text(
              l.text('estimate', {
                'amount': '${formatMoney(estimate.receive)} $receive',
              }),
              style: const TextStyle(fontSize: 16, color: KioskColors.muted),
            ),
          ],
          const SizedBox(height: 20),
          Text(
            l.text('payment'),
            style: const TextStyle(fontSize: 18, fontWeight: FontWeight.w700),
          ),
          const SizedBox(height: 8),
          DropdownButtonFormField<String>(
            key: ValueKey('payment-${s.type.wire}-${s.paymentMethod}'),
            initialValue: s.paymentMethod,
            isExpanded: true,
            hint: Text(l.text('choosePayment')),
            style: TextStyle(
              fontFamily: Theme.of(context).textTheme.bodyLarge?.fontFamily,
              fontFamilyFallback: Theme.of(
                context,
              ).textTheme.bodyLarge?.fontFamilyFallback,
              fontSize: 20,
              color: KioskColors.ink,
            ),
            items: [
              for (final m in s.availablePaymentMethods)
                DropdownMenuItem(value: m, child: Text(paymentName(l, m))),
            ],
            onChanged: c.setPaymentMethod,
          ),
          // The owner's instruction for the chosen channel (v2 `kiosk.js:51`).
          if (s.paymentInstruction.isNotEmpty) ...[
            const SizedBox(height: 8),
            Text(
              s.paymentInstruction,
              key: const Key('payment-instruction'),
              style: const TextStyle(
                fontSize: 17,
                fontWeight: FontWeight.w700,
                color: KioskColors.ink,
              ),
            ),
          ],
          const SizedBox(height: 6),
          Text(
            l.text('callAgent'),
            style: const TextStyle(fontSize: 15, color: KioskColors.muted),
          ),
          const SizedBox(height: 24),
          PrimaryButton(
            key: const Key('review'),
            label: l.text('review'),
            onPressed: s.canReview ? c.submitQuote : null,
          ),
        ],
      ),
    );
  }

  Widget _freshnessBlock(RateFreshness freshness) {
    final formattedTime = freshness.publishedAt != null
        ? _formatBangkokTime(freshness.publishedAt!)
        : null;

    return Container(
      key: const Key('rate-freshness'),
      child: Wrap(
        spacing: 8,
        runSpacing: 4,
        crossAxisAlignment: WrapCrossAlignment.center,
        children: [
          if (freshness.stale)
            Container(
              key: const Key('rate-stale'),
              padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 2),
              decoration: BoxDecoration(
                color: const Color(0xFFFFEDD5),
                borderRadius: BorderRadius.circular(999),
              ),
              child: Text(
                l.text('rateStale'),
                style: const TextStyle(
                  fontSize: 12,
                  fontWeight: FontWeight.w600,
                  color: Color(0xFF9A3412),
                ),
              ),
            ),
          if (freshness.sourceDate != null)
            Text(
              l.text('rateSourceDate', {'date': freshness.sourceDate!}),
              style: const TextStyle(fontSize: 12, color: KioskColors.muted),
            )
          else
            Text(
              l.text('rateNoSourceDate'),
              style: const TextStyle(fontSize: 12, color: KioskColors.muted),
            ),
          if (formattedTime != null)
            Text(
              l.text('ratePublishedAt', {'at': formattedTime}),
              style: const TextStyle(fontSize: 12, color: KioskColors.muted),
            ),
        ],
      ),
    );
  }

  String _formatBangkokTime(DateTime utcTime) {
    final bkk = utcTime.toUtc().add(const Duration(hours: 7));
    final y = bkk.year.toString().padLeft(4, '0');
    final m = bkk.month.toString().padLeft(2, '0');
    final d = bkk.day.toString().padLeft(2, '0');
    final h = bkk.hour.toString().padLeft(2, '0');
    final min = bkk.minute.toString().padLeft(2, '0');
    return '$y-$m-$d $h:$min';
  }
}

class _DirectionButton extends StatelessWidget {
  const _DirectionButton({
    required this.title,
    required this.hint,
    required this.selected,
    required this.onTap,
  });
  final String title, hint;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final fg = selected ? Colors.white : KioskColors.ink;
    return Semantics(
      button: true,
      selected: selected,
      child: Material(
        color: selected ? KioskColors.brand : Colors.white,
        borderRadius: BorderRadius.circular(18),
        child: InkWell(
          borderRadius: BorderRadius.circular(18),
          onTap: onTap,
          child: Container(
            constraints: const BoxConstraints(minHeight: 110),
            padding: const EdgeInsets.all(22),
            decoration: BoxDecoration(
              borderRadius: BorderRadius.circular(18),
              border: Border.all(
                color: selected ? KioskColors.brand : KioskColors.line,
                width: 2,
              ),
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              mainAxisAlignment: MainAxisAlignment.center,
              children: [
                Text(
                  title,
                  style: TextStyle(
                    fontSize: 24,
                    fontWeight: FontWeight.w800,
                    color: fg,
                  ),
                ),
                const SizedBox(height: 6),
                Text(hint, style: TextStyle(fontSize: 16, color: fg)),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

class _CurrencyChip extends StatelessWidget {
  const _CurrencyChip({
    required this.code,
    required this.label,
    required this.name,
    required this.selected,
    required this.onTap,
  });
  final String code, label, name;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) => Semantics(
    button: true,
    selected: selected,
    child: Material(
      color: selected ? KioskColors.soft : Colors.white,
      borderRadius: BorderRadius.circular(14),
      child: InkWell(
        key: Key('currency-$code'),
        borderRadius: BorderRadius.circular(14),
        onTap: onTap,
        child: Container(
          width: 150,
          padding: const EdgeInsets.symmetric(vertical: 14, horizontal: 10),
          decoration: BoxDecoration(
            borderRadius: BorderRadius.circular(14),
            border: Border.all(
              color: selected ? KioskColors.brand : KioskColors.line,
              width: 2,
            ),
          ),
          child: Column(
            children: [
              Ltr(
                label,
                style: TextStyle(
                  fontSize: 22,
                  fontWeight: FontWeight.w800,
                  color: selected ? KioskColors.brand : KioskColors.ink,
                ),
              ),
              const SizedBox(height: 4),
              Text(
                name,
                maxLines: 2,
                overflow: TextOverflow.ellipsis,
                textAlign: TextAlign.center,
                style: const TextStyle(fontSize: 13, color: KioskColors.muted),
              ),
            ],
          ),
        ),
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
