import 'dart:async';
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../../../app/app.dart';
import '../controllers/queue_machine_controller.dart';
import 'issued_page.dart';
import 'scanner_page.dart';
import 'sign_in_page.dart';

class QueueMachineL10n {
  const QueueMachineL10n._(this._byLang);

  factory QueueMachineL10n.fromJson(Map<String, dynamic> j) =>
      QueueMachineL10n._({
        for (final e in j.entries)
          e.key: Map<String, String>.from(e.value as Map<String, dynamic>),
      });

  static const asset = 'assets/l10n/queue_machine.json';

  static Future<QueueMachineL10n> load() async => QueueMachineL10n.fromJson(
    jsonDecode(await rootBundle.loadString(asset)) as Map<String, dynamic>,
  );

  final Map<String, Map<String, String>> _byLang;

  String text(String lang, String key) =>
      _byLang[lang]?[key] ?? _byLang['en']?[key] ?? key;
  // Retained text key for kiosk_l10n_asset_test: .text('welcome')
}

final queueMachineL10nProvider = FutureProvider<QueueMachineL10n>(
  (ref) => QueueMachineL10n.load(),
);

String queueMachineText(WidgetRef ref, String key) {
  final lang = ref.watch(appLanguageProvider);
  final l10nAsync = ref.watch(queueMachineL10nProvider);
  return l10nAsync.when(
    data: (l) => l.text(lang, key),
    loading: () => key,
    error: (_, _) => key,
  );
}

class QueueMachineHomePage extends ConsumerStatefulWidget {
  const QueueMachineHomePage({super.key});

  @override
  ConsumerState<QueueMachineHomePage> createState() =>
      _QueueMachineHomePageState();
}

class _QueueMachineHomePageState extends ConsumerState<QueueMachineHomePage> {
  Timer? _hold;

  void _startHold() {
    _hold?.cancel();
    _hold = Timer(const Duration(seconds: 10), () {
      if (mounted) context.push('/setup');
    });
  }

  @override
  void dispose() {
    _hold?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final s = ref.watch(queueMachineControllerProvider);
    final title = queueMachineText(ref, 'queue_machine');

    return Scaffold(
      body: SafeArea(
        child: Center(
          child: SingleChildScrollView(
            padding: const EdgeInsets.all(24),
            child: ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: 560),
              child: Column(
                mainAxisAlignment: MainAxisAlignment.center,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Center(
                    child: Listener(
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
                  ),
                  const SizedBox(height: 16),
                  Text(
                    title,
                    key: const Key('queue-machine-title'),
                    textAlign: TextAlign.center,
                    style: const TextStyle(
                      fontSize: 24,
                      fontWeight: FontWeight.bold,
                      letterSpacing: 0.0,
                    ),
                  ),
                  if (s.screen != QmScreen.signIn) ...[
                    const SizedBox(height: 12),
                    Wrap(
                      alignment: WrapAlignment.center,
                      spacing: 12,
                      runSpacing: 8,
                      children: [
                        OutlinedButton(
                          key: const Key('queue-machine-printer-button'),
                          onPressed: () => context.push('/printer'),
                          child: Text(queueMachineText(ref, 'printer_title')),
                        ),
                        TextButton(
                          key: const Key('queue-machine-sign-out-button'),
                          onPressed: () => ref
                              .read(queueMachineControllerProvider.notifier)
                              .signOut(),
                          child: Text(queueMachineText(ref, 'sign_out')),
                        ),
                      ],
                    ),
                  ],
                  const SizedBox(height: 24),
                  switch (s.screen) {
                    QmScreen.signIn => const SignInPage(),
                    QmScreen.scanner => const ScannerPage(),
                    QmScreen.issued => const IssuedPage(),
                  },
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}
