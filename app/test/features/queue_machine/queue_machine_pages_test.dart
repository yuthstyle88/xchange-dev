import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:cookie_jar/cookie_jar.dart';
import 'package:dio/dio.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:xchange_kiosk/app/app.dart';
import 'package:xchange_kiosk/app/router/app_router.dart';
import 'package:xchange_kiosk/core/endpoint/endpoint_store.dart';
import 'package:xchange_kiosk/core/endpoint/kiosk_endpoint.dart';
import 'package:xchange_kiosk/core/network/dio_provider.dart';
import 'package:xchange_kiosk/features/queue_machine/infrastructure/queue_machine_api.dart';
import 'package:xchange_kiosk/features/queue_machine/infrastructure/queue_machine_api_dio.dart';
import 'package:xchange_kiosk/features/queue_machine/infrastructure/staff_session.dart';
import 'package:xchange_kiosk/features/queue_machine/presentation/controllers/queue_machine_controller.dart';
import 'package:xchange_kiosk/features/queue_machine/presentation/pages/issued_page.dart';
import 'package:xchange_kiosk/features/queue_machine/presentation/pages/printer_page.dart';
import 'package:xchange_kiosk/features/queue_machine/presentation/pages/queue_machine_home_page.dart';
import 'package:xchange_kiosk/features/queue_machine/presentation/pages/scanner_page.dart';
import 'package:xchange_kiosk/features/queue_machine/presentation/pages/sign_in_page.dart';
import 'package:xchange_kiosk/features/queue_machine/printing/printer_providers.dart';
import 'package:xchange_kiosk/features/queue_machine/printing/ticket_printer.dart';
import 'package:xchange_kiosk/features/queue_machine/printing/usb_printer_port.dart';
import 'package:xchange_kiosk/features/setup/app/setup_providers.dart';

class _FakeApi implements QueueMachineApi {
  final signIns = <String>[];
  final issued = <String>[];
  Object? issueError;
  int seconds = 10;
  final bytes = Uint8List.fromList([0x1B, 0x40, 0x4B, 0x30, 0x34, 0x32, 0x0A]);

  @override
  Future<void> signIn(String identifier, String password) async {
    signIns.add(identifier);
  }

  @override
  Future<IssuedTicket> issueByToken(String token) async {
    issued.add(token);
    final e = issueError;
    if (e != null) throw e;
    return IssuedTicket(
      displayNumber: 'K042',
      ticketId: 't-1',
      ticketEscpos: base64Encode(bytes),
    );
  }

  @override
  Future<int> resultSeconds() async => seconds;
}

class _FakePrinter implements TicketPrinter {
  _FakePrinter({this.error, this.hold});
  final Object? error;
  final Completer<void>? hold;
  final writes = <Uint8List>[];

  @override
  Future<void> write(Uint8List bytes) async {
    final e = error;
    if (e != null) throw e;
    writes.add(Uint8List.fromList(bytes));
    final h = hold;
    if (h != null) await h.future;
  }
}

class _FakeUsbPort implements UsbPrinterPort {
  @override
  Future<List<UsbPrinterDevice>> devices() async => const [];

  @override
  Future<bool> hasPermission(UsbPrinterDevice device) async => true;

  @override
  Future<bool> requestPermission(UsbPrinterDevice device) async => true;

  @override
  Future<void> writeRaw(UsbPrinterDevice device, Uint8List bytes) async {}
}

class _MemoryEndpointStore implements EndpointStore {
  _MemoryEndpointStore(this._stored);
  KioskEndpoint? _stored;

  @override
  Future<KioskEndpoint?> load() async => _stored;

  @override
  Future<void> save(KioskEndpoint endpoint) async {
    _stored = endpoint;
  }

  @override
  Future<void> clear() async {
    _stored = null;
  }

  @override
  String? originHeader(KioskEndpoint endpoint) => endpoint.origin;
}

class _FakeStaffSession extends StaffSession {
  _FakeStaffSession() : super(Dio());
  int signOuts = 0;

  @override
  Future<void> signOut() async {
    signOuts += 1;
  }
}

class _StubAdapter implements HttpClientAdapter {
  _StubAdapter(this._handler);
  final ResponseBody Function(RequestOptions options) _handler;

  @override
  Future<ResponseBody> fetch(
    RequestOptions options,
    Stream<Uint8List>? requestStream,
    Future<void>? cancelFuture,
  ) async => _handler(options);

  @override
  void close({bool force = false}) {}
}

void main() {
  setUp(() {
    SharedPreferences.setMockInitialValues({});
  });

  Future<ProviderContainer> pumpMachine(
    WidgetTester tester, {
    required _FakeApi api,
    required _FakePrinter printer,
    String lang = 'en',
  }) async {
    tester.view.physicalSize = const Size(1280, 1000);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);

    final l10n = QueueMachineL10n.fromJson(
      jsonDecode(File(QueueMachineL10n.asset).readAsStringSync())
          as Map<String, dynamic>,
    );
    final endpoint = KioskEndpoint.parse('https://xc.example.com', 'MAIN');
    final container = ProviderContainer(
      overrides: [
        endpointStoreProvider.overrideWithValue(_MemoryEndpointStore(endpoint)),
        initialKioskEndpointProvider.overrideWithValue(endpoint),
        setupAvailableProvider.overrideWithValue(true),
        queueMachineL10nProvider.overrideWith((ref) => l10n),
        queueMachineApiProvider.overrideWithValue(api),
        staffSessionProvider.overrideWithValue(_FakeStaffSession()),
        ticketPrinterProvider.overrideWithValue(printer),
        usbPrinterPortProvider.overrideWithValue(_FakeUsbPort()),
      ],
    );
    container.read(appLanguageProvider.notifier).state = lang;

    await tester.pumpWidget(
      UncontrolledProviderScope(container: container, child: const KioskApp()),
    );
    await tester.pump();
    await tester.pump();
    return container;
  }

  testWidgets(
    'sign-in -> keyboard-wedge scan -> issued page with printed status -> auto-return after result_seconds',
    (tester) async {
      final api = _FakeApi()..seconds = 5;
      final printer = _FakePrinter();
      final container = await pumpMachine(tester, api: api, printer: printer);

      expect(find.byType(SignInPage), findsOneWidget);
      expect(find.byType(ScannerPage), findsNothing);

      await tester.enterText(
        find.byKey(const Key('sign-in-identifier')),
        'teller1',
      );
      await tester.enterText(
        find.byKey(const Key('sign-in-password')),
        'secret',
      );
      await tester.tap(find.byKey(const Key('sign-in-submit')));
      await tester.pump();
      await tester.pump();

      expect(api.signIns, ['teller1']);
      expect(find.byType(ScannerPage), findsOneWidget);

      await tester.enterText(
        find.byKey(const Key('scanner-input')),
        'https://xc.example.com/q#tok-123',
      );
      await tester.tap(find.byKey(const Key('scanner-submit')));
      await tester.pump();
      await tester.pump();

      expect(api.issued, ['tok-123']);
      expect(find.byType(IssuedPage), findsOneWidget);
      expect(
        tester.widget<Text>(find.byKey(const Key('issued-number'))).data,
        'K042',
      );
      expect(
        tester.widget<Text>(find.byKey(const Key('issued-print-status'))).data,
        'Printed',
      );

      await tester.pump(const Duration(seconds: 5));
      await tester.pump();
      expect(find.byType(ScannerPage), findsOneWidget);

      await tester.pumpWidget(const SizedBox());
      container.dispose();
    },
  );

  testWidgets(
    'issued page shows failed and noPrinter states, routes to PrinterPage, and signs out',
    (tester) async {
      final api = _FakeApi()..seconds = 30;
      final printer = _FakePrinter(
        error: const PrinterUnavailable('writeFailed'),
      );
      final container = await pumpMachine(tester, api: api, printer: printer);

      await tester.enterText(
        find.byKey(const Key('sign-in-identifier')),
        'teller1',
      );
      await tester.enterText(
        find.byKey(const Key('sign-in-password')),
        'secret',
      );
      await tester.tap(find.byKey(const Key('sign-in-submit')));
      await tester.pump();
      await tester.pump();

      await tester.enterText(
        find.byKey(const Key('scanner-input')),
        'https://xc.example.com/q#tok-1',
      );
      await tester.tap(find.byKey(const Key('scanner-submit')));
      await tester.pump();
      await tester.pump();

      expect(
        tester.widget<Text>(find.byKey(const Key('issued-print-status'))).data,
        'Print failed — scan again',
      );

      // Re-scan while on the issued screen issues again (§3.3, §4 D5).
      await tester.enterText(
        find.byKey(const Key('issued-scan-input')),
        'https://xc.example.com/q#tok-1',
      );
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pump();
      await tester.pump();
      expect(api.issued, ['tok-1', 'tok-1']);

      // Open printer settings from the machine's menu.
      await tester.tap(find.byKey(const Key('queue-machine-printer-button')));
      await tester.pump();
      await tester.pump();
      expect(find.byType(PrinterPage), findsOneWidget);

      // Navigate back and sign out.
      container.read(appRouterProvider).go('/');
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 500));

      await tester.tap(find.byKey(const Key('queue-machine-sign-out-button')));
      await tester.pump();
      await tester.pump();
      expect(find.byType(SignInPage), findsOneWidget);

      await tester.pumpWidget(const SizedBox());
      container.dispose();
    },
  );

  test(
    'W6 + W8 + W12: DioQueueMachineApi and StaffSession persist cookie, send Origin and X-Xchange-Request, and issueByToken',
    () async {
      SharedPreferences.setMockInitialValues({});
      final requests = <RequestOptions>[];
      final jar = PersistCookieJar(
        storage: const SharedPreferencesCookieStorage(),
      );
      final dio = buildKioskDio(
        baseUrl: 'https://xc.example.com',
        origin: 'https://xc.example.com',
        cookieJar: jar,
      );
      dio.httpClientAdapter = _StubAdapter((options) {
        requests.add(options);
        if (options.path == '/api/auth/login') {
          return ResponseBody.fromString(
            jsonEncode({
              'user': {'id': 'u-1'},
            }),
            200,
            headers: {
              Headers.contentTypeHeader: [Headers.jsonContentType],
              'set-cookie': [
                '__Host-xc_session=sess-abc; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=43200',
              ],
            },
          );
        }
        if (options.path == '/api/auth/refresh') {
          return ResponseBody.fromString(
            jsonEncode({'ok': true}),
            200,
            headers: {
              Headers.contentTypeHeader: [Headers.jsonContentType],
            },
          );
        }
        if (options.path == '/api/auth/logout') {
          return ResponseBody.fromString(
            jsonEncode({'ok': true}),
            200,
            headers: {
              Headers.contentTypeHeader: [Headers.jsonContentType],
            },
          );
        }
        if (options.path == '/api/kiosk') {
          final body = options.data as Map<String, dynamic>;
          if (body['action'] == 'issueByToken') {
            return ResponseBody.fromString(
              jsonEncode({
                'displayNumber': 'K042',
                'ticketId': 't-42',
                'ticketEscpos': 'G0BLMDQyCg==',
              }),
              200,
              headers: {
                Headers.contentTypeHeader: [Headers.jsonContentType],
              },
            );
          }
          if (body['action'] == 'settings') {
            return ResponseBody.fromString(
              jsonEncode({'resultSeconds': 12}),
              200,
              headers: {
                Headers.contentTypeHeader: [Headers.jsonContentType],
              },
            );
          }
        }
        return ResponseBody.fromString(
          jsonEncode({'error': 'notFound'}),
          404,
          headers: {
            Headers.contentTypeHeader: [Headers.jsonContentType],
          },
        );
      });

      final session = StaffSession(dio, cookieJar: jar);
      final api = DioQueueMachineApi(
        dio: dio,
        session: session,
        branchId: () => 'MAIN',
      );

      await api.signIn('teller1', 'pw');
      final ticket = await api.issueByToken('tok-99');
      expect(ticket.displayNumber, 'K042');
      expect(ticket.ticketId, 't-42');
      expect(await api.resultSeconds(), 12);

      // Verify headers and cookie jar on issueByToken call.
      final issueReq = requests[1];
      expect(issueReq.headers['X-Xchange-Request'], '1');
      expect(issueReq.headers['Origin'], 'https://xc.example.com');
      expect(
        issueReq.headers['cookie'],
        contains('__Host-xc_session=sess-abc'),
      );
      expect(issueReq.data, {
        'action': 'issueByToken',
        'payload': {'branchId': 'MAIN', 'token': 'tok-99'},
      });

      await session.refresh();
      await session.signOut();
      final cookiesAfterLogout = await jar.loadForRequest(
        Uri.parse('https://xc.example.com/api/kiosk'),
      );
      expect(cookiesAfterLogout, isEmpty);
    },
  );

  test(
    'second scan of the same token while the write is pending gives writes == 1',
    () async {
      final api = _FakeApi();
      final hold = Completer<void>();
      final printer = _FakePrinter(hold: hold);
      final container = ProviderContainer(
        overrides: [
          queueMachineApiProvider.overrideWithValue(api),
          staffSessionProvider.overrideWithValue(_FakeStaffSession()),
          ticketPrinterProvider.overrideWithValue(printer),
        ],
      );
      addTearDown(container.dispose);
      container.listen(queueMachineControllerProvider, (_, _) {});

      final ctl = container.read(queueMachineControllerProvider.notifier);
      await ctl.signIn('teller1', 'secret');
      expect(
        container.read(queueMachineControllerProvider).screen,
        QmScreen.scanner,
      );

      final first = ctl.onScan('https://h/q#tok');
      await Future<void>.delayed(Duration.zero);
      expect(printer.writes, hasLength(1));

      // Second scan of the same token while write is pending is dropped.
      await ctl.onScan('https://h/q#tok');
      expect(printer.writes, hasLength(1));
      expect(api.issued, ['tok']);

      hold.complete();
      await first;
      expect(printer.writes, hasLength(1));
      expect(
        container.read(queueMachineControllerProvider).screen,
        QmScreen.issued,
      );
    },
  );

  test(
    '401 from issueByToken sends the machine to signIn, never scanner',
    () async {
      for (final err in const [
        QueueMachineRefused('signIn', statusCode: 401),
        QueueMachineRefused('EMPLOYEE_AUTH_REQUIRED', statusCode: 401),
        QueueMachineRefused('signIn'),
        QueueMachineRefused('EMPLOYEE_AUTH_REQUIRED'),
      ]) {
        final api = _FakeApi()..issueError = err;
        final printer = _FakePrinter();
        final container = ProviderContainer(
          overrides: [
            queueMachineApiProvider.overrideWithValue(api),
            staffSessionProvider.overrideWithValue(_FakeStaffSession()),
            ticketPrinterProvider.overrideWithValue(printer),
          ],
        );
        container.listen(queueMachineControllerProvider, (_, _) {});

        final ctl = container.read(queueMachineControllerProvider.notifier);
        await ctl.signIn('teller1', 'secret');
        expect(
          container.read(queueMachineControllerProvider).screen,
          QmScreen.scanner,
        );

        await ctl.onScan('https://h/q#tok');
        expect(
          container.read(queueMachineControllerProvider).screen,
          QmScreen.signIn,
        );
        expect(container.read(queueMachineControllerProvider).error, isNull);
        expect(printer.writes, isEmpty);
        container.dispose();
      }
    },
  );

  test(
    'on 401 DioQueueMachineApi calls refresh() once and retries; if refresh also answers 401, controller goes to signIn',
    () async {
      SharedPreferences.setMockInitialValues({});
      var issueCalls = 0;
      var refreshCalls = 0;
      var refreshStatus = 200;

      final jar = PersistCookieJar(
        storage: const SharedPreferencesCookieStorage(),
      );
      final dio = buildKioskDio(
        baseUrl: 'https://xc.example.com',
        origin: 'https://xc.example.com',
        cookieJar: jar,
      );
      dio.httpClientAdapter = _StubAdapter((options) {
        if (options.path == '/api/auth/login') {
          return ResponseBody.fromString(
            jsonEncode({
              'user': {'id': 'u-1'},
            }),
            200,
            headers: {
              Headers.contentTypeHeader: [Headers.jsonContentType],
            },
          );
        }
        if (options.path == '/api/auth/refresh') {
          refreshCalls += 1;
          return ResponseBody.fromString(
            jsonEncode(
              refreshStatus == 200
                  ? {'ok': true}
                  : {
                      'error': {'code': 'EMPLOYEE_AUTH_REQUIRED'},
                    },
            ),
            refreshStatus,
            headers: {
              Headers.contentTypeHeader: [Headers.jsonContentType],
            },
          );
        }
        if (options.path == '/api/kiosk') {
          final body = options.data as Map<String, dynamic>;
          if (body['action'] == 'issueByToken') {
            issueCalls += 1;
            if (issueCalls == 1 || refreshStatus != 200) {
              return ResponseBody.fromString(
                jsonEncode({
                  'error': {'code': 'signIn'},
                }),
                401,
                headers: {
                  Headers.contentTypeHeader: [Headers.jsonContentType],
                },
              );
            }
            return ResponseBody.fromString(
              jsonEncode({
                'displayNumber': 'K042',
                'ticketId': 't-42',
                'ticketEscpos': 'G0BLMDQyCg==',
              }),
              200,
              headers: {
                Headers.contentTypeHeader: [Headers.jsonContentType],
              },
            );
          }
          if (body['action'] == 'settings') {
            return ResponseBody.fromString(
              jsonEncode({'resultSeconds': 10}),
              200,
              headers: {
                Headers.contentTypeHeader: [Headers.jsonContentType],
              },
            );
          }
        }
        return ResponseBody.fromString('{}', 404);
      });

      final session = StaffSession(dio, cookieJar: jar);
      final api = DioQueueMachineApi(
        dio: dio,
        session: session,
        branchId: () => 'MAIN',
      );
      final printer = _FakePrinter();
      final container = ProviderContainer(
        overrides: [
          queueMachineApiProvider.overrideWithValue(api),
          staffSessionProvider.overrideWithValue(session),
          ticketPrinterProvider.overrideWithValue(printer),
        ],
      );
      addTearDown(container.dispose);
      container.listen(queueMachineControllerProvider, (_, _) {});

      final ctl = container.read(queueMachineControllerProvider.notifier);
      await ctl.signIn('teller1', 'pw');

      // Case 1: first issueByToken returns 401 -> refresh() succeeds -> retry succeeds.
      await ctl.onScan('https://xc.example.com/q#tok-1');
      expect(refreshCalls, 1);
      expect(issueCalls, 2);
      expect(
        container.read(queueMachineControllerProvider).screen,
        QmScreen.issued,
      );
      expect(
        container.read(queueMachineControllerProvider).displayNumber,
        'K042',
      );

      // Case 2: issueByToken returns 401 and refresh() also returns 401 -> goes to signIn.
      issueCalls = 0;
      refreshCalls = 0;
      refreshStatus = 401;
      await ctl.onScan('https://xc.example.com/q#tok-2');
      expect(refreshCalls, 1);
      expect(issueCalls, 1);
      expect(
        container.read(queueMachineControllerProvider).screen,
        QmScreen.signIn,
      );
    },
  );
}
