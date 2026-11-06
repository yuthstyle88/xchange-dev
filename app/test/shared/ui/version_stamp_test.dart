import 'dart:convert';
import 'dart:typed_data';

import 'package:dio/dio.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/core/network/dio_provider.dart';
import 'package:xchange_kiosk/features/queue_machine/presentation/controllers/queue_machine_controller.dart';
import 'package:xchange_kiosk/features/queue_machine/presentation/pages/queue_machine_home_page.dart';
import 'package:xchange_kiosk/features/queue_machine/presentation/pages/sign_in_page.dart';
import 'package:xchange_kiosk/shared/ui/version_stamp.dart';

class _FakeDioAdapter implements HttpClientAdapter {
  _FakeDioAdapter({this.handler});

  final Future<ResponseBody> Function(RequestOptions options)? handler;

  @override
  Future<ResponseBody> fetch(
    RequestOptions options,
    Stream<Uint8List>? requestStream,
    Future<void>? cancelFuture,
  ) async {
    if (handler != null) {
      return handler!(options);
    }
    return ResponseBody.fromString(
      jsonEncode({
        'version': '1.0.0',
        'build': 'bac76f782',
        'builtAt': '2026-11-05T00:00:00Z',
        'channel': 'staging',
      }),
      200,
      headers: {
        Headers.contentTypeHeader: [Headers.jsonContentType],
      },
    );
  }

  @override
  void close({bool force = false}) {}
}

Dio _createDio({_FakeDioAdapter? adapter}) {
  final dio = Dio(BaseOptions(baseUrl: 'https://kiosk.example.com'));
  dio.httpClientAdapter = adapter ?? _FakeDioAdapter();
  return dio;
}

class _TestQmController extends QueueMachineController {
  _TestQmController([this._initialState = const QueueMachineState()]);
  final QueueMachineState _initialState;

  @override
  QueueMachineState build() => _initialState;
}

void main() {
  final emptyL10n = QueueMachineL10n.fromJson(const {});

  group('V1: All four defines set, server ok', () {
    testWidgets('renders full stamp with server version', (tester) async {
      final dio = _createDio();
      await tester.pumpWidget(
        ProviderScope(
          overrides: [dioProvider.overrideWithValue(dio)],
          child: const MaterialApp(
            home: Scaffold(
              body: VersionStamp(
                appVersion: '1.0.0+1',
                buildSha: 'abcdef1',
                builtAt: '2026-11-05T00:00:00Z',
                channel: 'staging',
              ),
            ),
          ),
        ),
      );

      await tester.pumpAndSettle();

      expect(
        find.text('v1.0.0+1 · abcdef1 · staging · server v1.0.0 · bac76f7'),
        findsOneWidget,
      );
    });
  });

  group('V2: No defines', () {
    testWidgets('renders unknown placeholders with server version', (
      tester,
    ) async {
      final dio = _createDio();
      await tester.pumpWidget(
        ProviderScope(
          overrides: [dioProvider.overrideWithValue(dio)],
          child: const MaterialApp(home: Scaffold(body: VersionStamp())),
        ),
      );

      await tester.pumpAndSettle();

      expect(
        find.text('vunknown · unknown · unknown · server v1.0.0 · bac76f7'),
        findsOneWidget,
      );
    });
  });

  group('V3: /health/ready fails', () {
    testWidgets('renders server ? when request fails', (tester) async {
      final failingAdapter = _FakeDioAdapter(
        handler: (options) async => ResponseBody.fromString(
          'Internal Server Error',
          500,
          headers: {
            Headers.contentTypeHeader: ['text/plain'],
          },
        ),
      );
      final dio = _createDio(adapter: failingAdapter);

      await tester.pumpWidget(
        ProviderScope(
          overrides: [dioProvider.overrideWithValue(dio)],
          child: const MaterialApp(
            home: Scaffold(
              body: VersionStamp(
                appVersion: '1.0.0+1',
                buildSha: 'abcdef1',
                channel: 'staging',
              ),
            ),
          ),
        ),
      );

      await tester.pumpAndSettle();

      expect(
        find.text('v1.0.0+1 · abcdef1 · staging · server ?'),
        findsOneWidget,
      );
    });

    testWidgets('renders server ? with no defines when request throws', (
      tester,
    ) async {
      final throwingAdapter = _FakeDioAdapter(
        handler: (options) async =>
            throw DioException(requestOptions: options, error: 'network down'),
      );
      final dio = _createDio(adapter: throwingAdapter);

      await tester.pumpWidget(
        ProviderScope(
          overrides: [dioProvider.overrideWithValue(dio)],
          child: const MaterialApp(home: Scaffold(body: VersionStamp())),
        ),
      );

      await tester.pumpAndSettle();

      expect(
        find.text('vunknown · unknown · unknown · server ?'),
        findsOneWidget,
      );
    });
  });

  group('V4: Sign-in page and home page', () {
    testWidgets('sign-in page renders the stamp', (tester) async {
      final dio = _createDio();
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            dioProvider.overrideWithValue(dio),
            queueMachineL10nProvider.overrideWith((ref) => emptyL10n),
            queueMachineControllerProvider.overrideWith(
              () => _TestQmController(
                const QueueMachineState(screen: QmScreen.signIn),
              ),
            ),
          ],
          child: const MaterialApp(home: Scaffold(body: SignInPage())),
        ),
      );

      await tester.pumpAndSettle();

      expect(
        find.descendant(
          of: find.byType(SignInPage),
          matching: find.byType(VersionStamp),
        ),
        findsOneWidget,
      );
    });

    testWidgets('home page renders the stamp when signed in', (tester) async {
      final dio = _createDio();
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            dioProvider.overrideWithValue(dio),
            queueMachineL10nProvider.overrideWith((ref) => emptyL10n),
            queueMachineControllerProvider.overrideWith(
              () => _TestQmController(
                const QueueMachineState(screen: QmScreen.issued),
              ),
            ),
          ],
          child: const MaterialApp(
            home: Scaffold(body: QueueMachineHomePage()),
          ),
        ),
      );

      await tester.pumpAndSettle();

      expect(
        find.descendant(
          of: find.byType(QueueMachineHomePage),
          matching: find.byType(VersionStamp),
        ),
        findsOneWidget,
      );
    });

    testWidgets(
      'home page renders exactly one stamp in default sign-in state',
      (tester) async {
        final dio = _createDio();
        await tester.pumpWidget(
          ProviderScope(
            overrides: [
              dioProvider.overrideWithValue(dio),
              queueMachineL10nProvider.overrideWith((ref) => emptyL10n),
              queueMachineControllerProvider.overrideWith(
                () => _TestQmController(
                  const QueueMachineState(screen: QmScreen.signIn),
                ),
              ),
            ],
            child: const MaterialApp(
              home: Scaffold(body: QueueMachineHomePage()),
            ),
          ),
        );

        await tester.pumpAndSettle();

        expect(
          find.descendant(
            of: find.byType(QueueMachineHomePage),
            matching: find.byType(VersionStamp),
          ),
          findsOneWidget,
        );
      },
    );
  });
}
