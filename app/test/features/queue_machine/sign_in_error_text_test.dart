import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/app/app.dart';
import 'package:xchange_kiosk/features/queue_machine/presentation/controllers/queue_machine_controller.dart';
import 'package:xchange_kiosk/features/queue_machine/presentation/pages/queue_machine_home_page.dart';
import 'package:xchange_kiosk/features/queue_machine/presentation/pages/sign_in_page.dart';

class _StubQueueMachineController extends QueueMachineController {
  _StubQueueMachineController(this._state);
  final QueueMachineState _state;

  @override
  QueueMachineState build() => _state;
}

void main() {
  late QueueMachineL10n l10n;

  setUpAll(() {
    final raw = File(QueueMachineL10n.asset).readAsStringSync();
    l10n = QueueMachineL10n.fromJson(jsonDecode(raw) as Map<String, dynamic>);
  });

  Future<void> pumpSignIn(
    WidgetTester tester, {
    String? error,
    required String lang,
  }) async {
    final container = ProviderContainer(
      overrides: [
        queueMachineL10nProvider.overrideWith((ref) => l10n),
        queueMachineControllerProvider.overrideWith(
          () => _StubQueueMachineController(QueueMachineState(error: error)),
        ),
      ],
    );
    container.read(appLanguageProvider.notifier).state = lang;
    addTearDown(container.dispose);

    await tester.pumpWidget(
      UncontrolledProviderScope(
        container: container,
        child: const MaterialApp(home: Scaffold(body: SignInPage())),
      ),
    );
    await tester.pump();
  }

  testWidgets('state.error = unauthorized -> sign_in_error in en and th', (
    tester,
  ) async {
    await pumpSignIn(tester, error: 'unauthorized', lang: 'en');
    expect(find.byKey(const Key('sign-in-error')), findsOneWidget);
    expect(
      tester.widget<Text>(find.byKey(const Key('sign-in-error'))).data,
      'Sign-in failed, please check your credentials',
    );

    await pumpSignIn(tester, error: 'unauthorized', lang: 'th');
    expect(find.byKey(const Key('sign-in-error')), findsOneWidget);
    expect(
      tester.widget<Text>(find.byKey(const Key('sign-in-error'))).data,
      'เข้าสู่ระบบไม่สำเร็จ กรุณาตรวจสอบข้อมูล',
    );
  });

  testWidgets(
    'state.error = invalidCredentials -> sign_in_error in en and th',
    (tester) async {
      await pumpSignIn(tester, error: 'invalidCredentials', lang: 'en');
      expect(find.byKey(const Key('sign-in-error')), findsOneWidget);
      expect(
        tester.widget<Text>(find.byKey(const Key('sign-in-error'))).data,
        'Sign-in failed, please check your credentials',
      );

      await pumpSignIn(tester, error: 'invalidCredentials', lang: 'th');
      expect(find.byKey(const Key('sign-in-error')), findsOneWidget);
      expect(
        tester.widget<Text>(find.byKey(const Key('sign-in-error'))).data,
        'เข้าสู่ระบบไม่สำเร็จ กรุณาตรวจสอบข้อมูล',
      );
    },
  );

  testWidgets(
    'state.error = networkError -> sign_in_unavailable in en and th',
    (tester) async {
      await pumpSignIn(tester, error: 'networkError', lang: 'en');
      expect(find.byKey(const Key('sign-in-error')), findsOneWidget);
      expect(
        tester.widget<Text>(find.byKey(const Key('sign-in-error'))).data,
        'The sign-in service is not answering. Please tell the administrator.',
      );

      await pumpSignIn(tester, error: 'networkError', lang: 'th');
      expect(find.byKey(const Key('sign-in-error')), findsOneWidget);
      expect(
        tester.widget<Text>(find.byKey(const Key('sign-in-error'))).data,
        'ระบบเข้าสู่ระบบไม่ตอบ กรุณาแจ้งผู้ดูแลระบบ',
      );
    },
  );

  testWidgets(
    'state.error = signInFailed -> sign_in_unavailable in en and th',
    (tester) async {
      await pumpSignIn(tester, error: 'signInFailed', lang: 'en');
      expect(find.byKey(const Key('sign-in-error')), findsOneWidget);
      expect(
        tester.widget<Text>(find.byKey(const Key('sign-in-error'))).data,
        'The sign-in service is not answering. Please tell the administrator.',
      );

      await pumpSignIn(tester, error: 'signInFailed', lang: 'th');
      expect(find.byKey(const Key('sign-in-error')), findsOneWidget);
      expect(
        tester.widget<Text>(find.byKey(const Key('sign-in-error'))).data,
        'ระบบเข้าสู่ระบบไม่ตอบ กรุณาแจ้งผู้ดูแลระบบ',
      );
    },
  );

  testWidgets('state.error = notFound -> sign_in_unavailable in en and th', (
    tester,
  ) async {
    await pumpSignIn(tester, error: 'notFound', lang: 'en');
    expect(find.byKey(const Key('sign-in-error')), findsOneWidget);
    expect(
      tester.widget<Text>(find.byKey(const Key('sign-in-error'))).data,
      'The sign-in service is not answering. Please tell the administrator.',
    );

    await pumpSignIn(tester, error: 'notFound', lang: 'th');
    expect(find.byKey(const Key('sign-in-error')), findsOneWidget);
    expect(
      tester.widget<Text>(find.byKey(const Key('sign-in-error'))).data,
      'ระบบเข้าสู่ระบบไม่ตอบ กรุณาแจ้งผู้ดูแลระบบ',
    );
  });

  testWidgets('state.error = null -> no error text in en and th', (
    tester,
  ) async {
    await pumpSignIn(tester, error: null, lang: 'en');
    expect(find.byKey(const Key('sign-in-error')), findsNothing);

    await pumpSignIn(tester, error: null, lang: 'th');
    expect(find.byKey(const Key('sign-in-error')), findsNothing);
  });
}
