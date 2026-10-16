import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../../../app/router/app_router.dart';
import '../../../../l10n/kiosk_l10n.dart';
import '../../../../shared/theme/colors.dart';
import '../../../setup/app/setup_providers.dart';
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
      final narrow = MediaQuery.sizeOf(context).width <= 760;
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
              body: DecoratedBox(
                decoration: const BoxDecoration(
                  gradient: RadialGradient(
                    center: Alignment(0.9, -0.9),
                    radius: 0.8,
                    colors: [Color(0xFFF7E6EE), Color(0x00F7E6EE)],
                  ),
                ),
                child: SafeArea(
                  child: Column(
                    children: [
                      const _Header(),
                      Expanded(
                        child: SingleChildScrollView(
                          // A new screen starts at its top.
                          key: ValueKey(s.screen),
                          padding: narrow
                              ? const EdgeInsets.fromLTRB(16, 22, 16, 36)
                              : const EdgeInsets.fromLTRB(32, 28, 32, 36),
                          child: Center(
                            child: ConstrainedBox(
                              constraints: const BoxConstraints(maxWidth: 1200),
                              child: Column(
                                crossAxisAlignment: CrossAxisAlignment.stretch,
                                children: [
                                  Text(
                                    l.text(step),
                                    style: const TextStyle(
                                      fontSize: 12,
                                      fontWeight: FontWeight.w700,
                                      color: KioskColors.brand,
                                      letterSpacing: 1.8,
                                    ),
                                  ),
                                  const SizedBox(height: 14),
                                  // #325: the title and subtitle ask the
                                  // customer to choose; only step 1 does.
                                  if (s.screen == KioskScreen.select) ...[
                                    Text(
                                      l.text('title'),
                                      style: TextStyle(
                                        fontSize:
                                            (MediaQuery.sizeOf(context).width *
                                                    0.04)
                                                .clamp(32, 48),
                                        fontWeight: FontWeight.w700,
                                        letterSpacing: -1.6,
                                        height: 1.12,
                                      ),
                                    ),
                                    const SizedBox(height: 10),
                                    Text(
                                      l.text('subtitle'),
                                      style: TextStyle(
                                        fontSize: narrow ? 17 : 18,
                                        fontWeight: FontWeight.w400,
                                        color: KioskColors.muted,
                                      ),
                                    ),
                                    const SizedBox(height: 24),
                                  ],
                                  // #326: above the step, so it is seen without scrolling.
                                  if (s.error.isNotEmpty) ...[
                                    Semantics(
                                      liveRegion: true,
                                      child: Container(
                                        padding: const EdgeInsets.all(16),
                                        decoration: BoxDecoration(
                                          color: KioskColors.soft,
                                          border: Border.all(
                                            color: const Color(0xFFEFB3C8),
                                          ),
                                          borderRadius: BorderRadius.circular(
                                            10,
                                          ),
                                        ),
                                        child: Text(
                                          s.error,
                                          key: const Key('error'),
                                          style: const TextStyle(
                                            fontSize: 18,
                                            fontWeight: FontWeight.normal,
                                            color: Color(0xFF870031),
                                          ),
                                        ),
                                      ),
                                    ),
                                    const SizedBox(height: 20),
                                  ],
                                  switch (s.screen) {
                                    KioskScreen.select => const SelectPage(),
                                    KioskScreen.quote => const QuotePage(),
                                    KioskScreen.waiting => const WaitingPage(),
                                    KioskScreen.receipt => const ReceiptPage(),
                                  },
                                  const SizedBox(height: 24),
                                  Text(
                                    l.text('footer'),
                                    textAlign: TextAlign.center,
                                    style: const TextStyle(
                                      fontSize: 12,
                                      height: 1.8,
                                      color: KioskColors.muted,
                                    ),
                                  ),
                                  const SizedBox(height: 24),
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
  static const setupHold = Duration(seconds: 10);

  final _search = TextEditingController();
  Timer? _hold;

  void _startHold() {
    _hold?.cancel();
    if (!ref.read(setupAvailableProvider)) return;
    _hold = Timer(setupHold, () {
      if (mounted) context.go(setupPath);
    });
  }

  @override
  void dispose() {
    _hold?.cancel();
    _search.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    ref.listen<KioskState>(kioskControllerProvider, (prev, next) {
      if (prev != null &&
          (prev.languageCode != next.languageCode ||
              prev.screen != next.screen)) {
        setState(_search.clear);
      }
    });
    final s = ref.watch(kioskControllerProvider);
    final c = ref.read(kioskControllerProvider.notifier);
    final l = ref.watch(kioskTextsProvider);
    final List<KioskLanguage> matches = l.search(_search.text);
    final narrow = MediaQuery.sizeOf(context).width <= 760;

    final searchField = TextField(
      controller: _search,
      onChanged: (_) => setState(() {}),
      style: const TextStyle(fontSize: 14),
      decoration: const InputDecoration(
        isDense: true,
        contentPadding: EdgeInsets.symmetric(horizontal: 12, vertical: 8),
        constraints: BoxConstraints(minHeight: 40),
        hintText: 'Search language / ภาษา / 国家',
      ),
    );

    final langDropdown = DropdownButtonFormField<String>(
      key: ValueKey('lang-${l.code}-${_search.text}'),
      initialValue: matches.any((x) => x.code == l.code) ? l.code : null,
      isDense: true,
      isExpanded: true,
      style: Theme.of(context).textTheme.titleMedium?.copyWith(fontSize: 14),
      decoration: const InputDecoration(
        isDense: true,
        contentPadding: EdgeInsets.symmetric(horizontal: 12, vertical: 8),
        constraints: BoxConstraints(minHeight: 40),
      ),
      items: [
        for (final x in matches)
          DropdownMenuItem(
            value: x.code,
            child: Text(x.label, style: const TextStyle(fontSize: 14)),
          ),
      ],
      onChanged: (code) {
        if (code != null) c.setLanguage(code);
      },
    );

    return Container(
      constraints: const BoxConstraints(minHeight: 98),
      decoration: const BoxDecoration(
        color: Colors.white,
        border: Border(bottom: BorderSide(color: KioskColors.line)),
      ),
      padding: narrow
          ? const EdgeInsets.symmetric(horizontal: 18, vertical: 10)
          : EdgeInsets.symmetric(
              horizontal: MediaQuery.sizeOf(context).width * 0.04,
              vertical: 12,
            ),
      child: narrow
          ? Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Row(
                  children: [
                    Listener(
                      key: const Key('kiosk-logo'),
                      onPointerDown: (_) => _startHold(),
                      onPointerUp: (_) => _hold?.cancel(),
                      onPointerCancel: (_) => _hold?.cancel(),
                      child: Image.asset(
                        'assets/img/logo.png',
                        width: 100,
                        height: 62,
                        fit: BoxFit.contain,
                        semanticLabel: 'XCHANGE Currency Exchange',
                      ),
                    ),
                    const Spacer(),
                    Flexible(
                      child: Text(
                        s.model?.branchName ?? l.text('welcome'),
                        key: const Key('branch'),
                        textAlign: TextAlign.end,
                        style: const TextStyle(
                          fontSize: 14,
                          fontWeight: FontWeight.w600,
                        ),
                      ),
                    ),
                  ],
                ),
                const SizedBox(height: 8),
                Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Text(
                      l.text('lang'),
                      style: const TextStyle(
                        fontSize: 12,
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                    const SizedBox(height: 4),
                    Row(
                      children: [
                        Expanded(child: searchField),
                        const SizedBox(width: 6),
                        Expanded(child: langDropdown),
                      ],
                    ),
                  ],
                ),
              ],
            )
          : Row(
              children: [
                // Ten seconds on the logo opens setup on an installed kiosk; a
                // kiosk has no menu, and a customer must not reach it by accident.
                Listener(
                  key: const Key('kiosk-logo'),
                  onPointerDown: (_) => _startHold(),
                  onPointerUp: (_) => _hold?.cancel(),
                  onPointerCancel: (_) => _hold?.cancel(),
                  child: Image.asset(
                    'assets/img/logo.png',
                    width: 130,
                    height: 76,
                    fit: BoxFit.contain,
                    semanticLabel: 'XCHANGE Currency Exchange',
                  ),
                ),
                const SizedBox(width: 18),
                Flexible(
                  child: Container(
                    constraints: const BoxConstraints(maxWidth: 160),
                    decoration: const BoxDecoration(
                      border: BorderDirectional(
                        start: BorderSide(color: KioskColors.line),
                      ),
                    ),
                    padding: const EdgeInsetsDirectional.only(start: 22),
                    child: const Text(
                      'T.K. EXCHANGE MONEY',
                      style: TextStyle(
                        fontSize: 10,
                        letterSpacing: 1.4,
                        color: KioskColors.muted,
                      ),
                    ),
                  ),
                ),
                const Spacer(),
                Flexible(
                  child: Text(
                    s.model?.branchName ?? l.text('welcome'),
                    key: const Key('branch'),
                    overflow: TextOverflow.ellipsis,
                    style: const TextStyle(
                      fontSize: 16,
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                ),
                const SizedBox(width: 16),
                SizedBox(
                  width: 270,
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      Text(
                        l.text('lang'),
                        style: const TextStyle(
                          fontSize: 12,
                          fontWeight: FontWeight.w600,
                        ),
                      ),
                      const SizedBox(height: 4),
                      searchField,
                      const SizedBox(height: 4),
                      langDropdown,
                    ],
                  ),
                ),
              ],
            ),
    );
  }
}
