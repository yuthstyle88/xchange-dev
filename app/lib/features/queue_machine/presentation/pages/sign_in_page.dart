import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../controllers/queue_machine_controller.dart';
import 'queue_machine_home_page.dart';

class SignInPage extends ConsumerStatefulWidget {
  const SignInPage({super.key});

  @override
  ConsumerState<SignInPage> createState() => _SignInPageState();
}

class _SignInPageState extends ConsumerState<SignInPage> {
  final TextEditingController _identifier = TextEditingController();
  final TextEditingController _password = TextEditingController();

  @override
  void dispose() {
    _identifier.dispose();
    _password.dispose();
    super.dispose();
  }

  void _submit() {
    ref
        .read(queueMachineControllerProvider.notifier)
        .signIn(_identifier.text, _password.text);
  }

  @override
  Widget build(BuildContext context) {
    final s = ref.watch(queueMachineControllerProvider);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text(
          queueMachineText(ref, 'sign_in_title'),
          key: const Key('sign-in-title'),
          style: const TextStyle(fontSize: 22, fontWeight: FontWeight.w700),
        ),
        const SizedBox(height: 16),
        Text(queueMachineText(ref, 'sign_in_identifier')),
        const SizedBox(height: 6),
        TextField(
          key: const Key('sign-in-identifier'),
          controller: _identifier,
          autocorrect: false,
          onSubmitted: (_) => _submit(),
        ),
        const SizedBox(height: 12),
        Text(queueMachineText(ref, 'sign_in_password')),
        const SizedBox(height: 6),
        TextField(
          key: const Key('sign-in-password'),
          controller: _password,
          obscureText: true,
          autocorrect: false,
          onSubmitted: (_) => _submit(),
        ),
        if (s.error != null) ...[
          const SizedBox(height: 12),
          Text(
            queueMachineText(ref, 'sign_in_error'),
            key: const Key('sign-in-error'),
            style: const TextStyle(fontSize: 16, color: Colors.red),
          ),
        ],
        const SizedBox(height: 16),
        FilledButton(
          key: const Key('sign-in-submit'),
          onPressed: _submit,
          child: Text(queueMachineText(ref, 'sign_in_submit')),
        ),
      ],
    );
  }
}
