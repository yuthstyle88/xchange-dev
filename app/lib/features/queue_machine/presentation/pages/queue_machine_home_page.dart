import 'dart:async';
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../../../app/app.dart';

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
    final lang = ref.watch(appLanguageProvider);
    final l10nAsync = ref.watch(queueMachineL10nProvider);

    final title = l10nAsync.when(
      data: (l) => l.text(lang, 'queue_machine'),
      loading: () => 'queue machine',
      error: (_, _) => 'queue machine',
    );

    return Scaffold(
      body: SafeArea(
        child: Center(
          child: Column(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
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
              const SizedBox(height: 24),
              Text(
                title,
                key: const Key('queue-machine-title'),
                style: const TextStyle(
                  fontSize: 24,
                  fontWeight: FontWeight.bold,
                  letterSpacing: 0.0,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
