import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../../l10n/kiosk_l10n.dart';
import '../../../../shared/theme/colors.dart';
import '../../app/kiosk_providers.dart';
import '../controllers/kiosk_controller.dart';
import 'quote_page.dart';
import 'receipt_page.dart';
import 'select_page.dart';
import 'waiting_page.dart';

/// The frame around the four screens: brand, branch, language, step, error
/// and footer.
class KioskPage extends ConsumerWidget {
  const KioskPage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    {
      final s = ref.watch(kioskControllerProvider);
      final c = ref.read(kioskControllerProvider.notifier);
      final l = ref.watch(kioskTextsProvider);
      final step = switch (s.screen) {
        KioskScreen.select => 'step1',
        KioskScreen.quote => 'step2',
        _ => 'step3',
      };
      return Directionality(
        textDirection: l.direction,
        child: Listener(
          onPointerDown: (_) => c.activity(),
          child: Focus(
            onKeyEvent: (_, _) {
              c.activity();
              return KeyEventResult.ignored;
            },
            child: Scaffold(
              backgroundColor: KioskColors.stage,
              body: SafeArea(
                child: Column(
                  children: [
                    const _Header(),
                    Expanded(
                      child: SingleChildScrollView(
                        // A new screen starts at its top.
                        key: ValueKey(s.screen),
                        padding: const EdgeInsets.fromLTRB(24, 20, 24, 32),
                        child: Center(
                          child: ConstrainedBox(
                            constraints: const BoxConstraints(maxWidth: 1100),
                            child: Column(
                              crossAxisAlignment: CrossAxisAlignment.stretch,
                              children: [
                                Text(
                                  l.text(step),
                                  style: const TextStyle(
                                    fontSize: 15,
                                    fontWeight: FontWeight.w700,
                                    color: KioskColors.brand,
                                    letterSpacing: 1,
                                  ),
                                ),
                                const SizedBox(height: 6),
                                Text(
                                  l.text('title'),
                                  style: const TextStyle(
                                    fontSize: 40,
                                    fontWeight: FontWeight.w800,
                                    height: 1.1,
                                  ),
                                ),
                                Text(
                                  l.text('subtitle'),
                                  style: const TextStyle(
                                    fontSize: 20,
                                    color: KioskColors.muted,
                                  ),
                                ),
                                const SizedBox(height: 20),
                                switch (s.screen) {
                                  KioskScreen.select => const SelectPage(),
                                  KioskScreen.quote => const QuotePage(),
                                  KioskScreen.waiting => const WaitingPage(),
                                  KioskScreen.receipt => const ReceiptPage(),
                                },
                                if (s.error.isNotEmpty) ...[
                                  const SizedBox(height: 16),
                                  Semantics(
                                    liveRegion: true,
                                    child: Text(
                                      s.error,
                                      key: const Key('error'),
                                      style: const TextStyle(
                                        fontSize: 20,
                                        fontWeight: FontWeight.w700,
                                        color: KioskColors.brand,
                                      ),
                                    ),
                                  ),
                                ],
                                const SizedBox(height: 28),
                                Text(
                                  l.text('footer'),
                                  textAlign: TextAlign.center,
                                  style: const TextStyle(
                                    fontSize: 14,
                                    color: KioskColors.muted,
                                  ),
                                ),
                              ],
                            ),
                          ),
                        ),
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ),
        ),
      );
    }
  }
}

class _Header extends ConsumerStatefulWidget {
  const _Header();

  @override
  ConsumerState<_Header> createState() => _HeaderState();
}

class _HeaderState extends ConsumerState<_Header> {
  final _search = TextEditingController();

  @override
  void dispose() {
    _search.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final s = ref.watch(kioskControllerProvider);
    final c = ref.read(kioskControllerProvider.notifier);
    final l = ref.watch(kioskTextsProvider);
    final List<KioskLanguage> matches = l.search(_search.text);
    return Container(
      color: Colors.white,
      padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 12),
      child: Row(
        children: [
          Image.asset(
            'assets/img/logo.png',
            height: 56,
            semanticLabel: 'XCHANGE Currency Exchange',
          ),
          const SizedBox(width: 14),
          const Flexible(
            child: Text(
              'T.K. EXCHANGE MONEY',
              style: TextStyle(
                fontSize: 12,
                letterSpacing: 2,
                color: KioskColors.muted,
              ),
            ),
          ),
          const Spacer(),
          Text(
            s.model?.branchName ?? l.text('welcome'),
            key: const Key('branch'),
            style: const TextStyle(fontSize: 18, fontWeight: FontWeight.w800),
          ),
          const SizedBox(width: 20),
          SizedBox(
            width: 260,
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(
                  l.text('lang'),
                  style: const TextStyle(
                    fontSize: 12,
                    fontWeight: FontWeight.w700,
                    color: KioskColors.brand,
                  ),
                ),
                TextField(
                  controller: _search,
                  onChanged: (_) => setState(() {}),
                  decoration: const InputDecoration(
                    isDense: true,
                    hintText: 'Search language / ภาษา / 国家',
                  ),
                ),
                const SizedBox(height: 6),
                DropdownButtonFormField<String>(
                  key: ValueKey('lang-${l.code}-${_search.text}'),
                  initialValue: matches.any((x) => x.code == l.code)
                      ? l.code
                      : null,
                  isDense: true,
                  isExpanded: true,
                  items: [
                    for (final x in matches)
                      DropdownMenuItem(value: x.code, child: Text(x.label)),
                  ],
                  onChanged: (code) {
                    if (code != null) c.setLanguage(code);
                  },
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}
