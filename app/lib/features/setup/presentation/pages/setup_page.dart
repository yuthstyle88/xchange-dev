import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../../../core/endpoint/endpoint_store.dart';
import '../../../../shared/theme/colors.dart';
import '../../../kiosk/app/kiosk_providers.dart';
import '../../l10n/setup_l10n.dart';
import '../controllers/setup_controller.dart';

/// Where an installed kiosk is told its server and branch (§8.1). Reached
/// when nothing is stored, or by holding the kiosk's logo for ten seconds.
class SetupPage extends ConsumerStatefulWidget {
  const SetupPage({super.key});

  @override
  ConsumerState<SetupPage> createState() => _SetupPageState();
}

class _SetupPageState extends ConsumerState<SetupPage> {
  late final TextEditingController _server;
  late final TextEditingController _branch;

  @override
  void initState() {
    super.initState();
    final current = ref.read(kioskEndpointProvider);
    _server = TextEditingController(text: current?.origin ?? '');
    _branch = TextEditingController(text: current?.branchId ?? '');
    // A result from an earlier visit is not a test of what is typed now.
    Future.microtask(() {
      if (mounted) ref.read(setupControllerProvider.notifier).forget();
    });
  }

  @override
  void dispose() {
    _server.dispose();
    _branch.dispose();
    super.dispose();
  }

  void _edited(String _) => ref.read(setupControllerProvider.notifier).forget();

  Future<void> _save() async {
    final ok = await ref
        .read(setupControllerProvider.notifier)
        .save(_server.text, _branch.text);
    if (ok && mounted) context.go('/');
  }

  @override
  Widget build(BuildContext context) {
    final texts = ref.watch(setupL10nProvider).value;
    if (texts == null) return const Scaffold(body: SizedBox.shrink());
    final lang = SetupL10n.pick(ref.watch(deviceLanguagesProvider));
    String t(String key) => texts.text(lang, key);

    final s = ref.watch(setupControllerProvider);
    final c = ref.read(setupControllerProvider.notifier);
    final hasEndpoint = ref.watch(kioskEndpointProvider) != null;
    final error =
        s.serverMessage ?? (s.errorKey == null ? null : t(s.errorKey!));

    return Scaffold(
      backgroundColor: Colors.white,
      body: SafeArea(
        child: Center(
          child: SingleChildScrollView(
            padding: const EdgeInsets.all(24),
            child: ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: 520),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Image.asset('assets/img/logo.png', height: 56),
                  const SizedBox(height: 16),
                  Text(
                    t('title'),
                    style: const TextStyle(
                      fontSize: 26,
                      fontWeight: FontWeight.w800,
                    ),
                  ),
                  const SizedBox(height: 20),
                  TextField(
                    key: const Key('setup-server'),
                    controller: _server,
                    onChanged: _edited,
                    keyboardType: TextInputType.url,
                    autocorrect: false,
                    decoration: InputDecoration(
                      labelText: t('server'),
                      hintText: 'https://xc.example.com',
                    ),
                  ),
                  const SizedBox(height: 12),
                  TextField(
                    key: const Key('setup-branch'),
                    controller: _branch,
                    onChanged: _edited,
                    autocorrect: false,
                    textCapitalization: TextCapitalization.characters,
                    decoration: InputDecoration(labelText: t('branch')),
                  ),
                  const SizedBox(height: 20),
                  OutlinedButton(
                    key: const Key('setup-test'),
                    onPressed: s.testing
                        ? null
                        : () => c.test(_server.text, _branch.text),
                    child: Text(s.testing ? t('testing') : t('test')),
                  ),
                  const SizedBox(height: 12),
                  if (s.tested != null && s.branchName != null)
                    Text(
                      '${t('connected')}: ${s.branchName}',
                      key: const Key('setup-result'),
                      style: const TextStyle(
                        fontSize: 18,
                        fontWeight: FontWeight.w700,
                        color: KioskColors.brand,
                      ),
                    ),
                  if (error != null)
                    Text(
                      error,
                      key: const Key('setup-error'),
                      style: const TextStyle(fontSize: 16, color: Colors.red),
                    ),
                  const SizedBox(height: 20),
                  FilledButton(
                    key: const Key('setup-save'),
                    onPressed: s.tested != null && !s.testing ? _save : null,
                    child: Text(t('save')),
                  ),
                  if (hasEndpoint) ...[
                    const SizedBox(height: 8),
                    TextButton(
                      key: const Key('setup-back'),
                      onPressed: () => context.go('/'),
                      child: Text(t('back')),
                    ),
                  ],
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}
