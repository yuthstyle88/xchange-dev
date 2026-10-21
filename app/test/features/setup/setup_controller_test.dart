// #174 (A3): what the setup screen does with a server address and a branch
// code. Written before the code.
import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:dio/dio.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/core/endpoint/endpoint_store.dart';
import 'package:xchange_kiosk/core/endpoint/kiosk_endpoint.dart';
import 'package:xchange_kiosk/core/network/dio_provider.dart';
import 'package:xchange_kiosk/features/setup/app/setup_providers.dart';
import 'package:xchange_kiosk/features/setup/presentation/controllers/setup_controller.dart';

/// Remembers what was saved; answers [stored] on load.
class MemoryStore implements EndpointStore {
  MemoryStore([this.stored]);
  KioskEndpoint? stored;
  int saves = 0;

  @override
  Future<KioskEndpoint?> load() async => stored;

  @override
  Future<void> save(KioskEndpoint endpoint) async {
    saves++;
    stored = endpoint;
  }

  @override
  Future<void> clear() async => stored = null;

  @override
  String? originHeader(KioskEndpoint endpoint) => endpoint.origin;
}

DioException _dioError(String message) => DioException(
  requestOptions: RequestOptions(path: '/health/ready'),
  response: message.isEmpty
      ? null
      : Response(
          requestOptions: RequestOptions(path: '/health/ready'),
          statusCode: 400,
          data: {'message': message},
        ),
  message: message,
);

class _Adapter implements HttpClientAdapter {
  RequestOptions? last;

  @override
  Future<ResponseBody> fetch(
    RequestOptions options,
    Stream<Uint8List>? requestStream,
    Future<void>? cancelFuture,
  ) async {
    last = options;
    return ResponseBody.fromString(
      jsonEncode({'status': 'ok'}),
      200,
      headers: {
        Headers.contentTypeHeader: [Headers.jsonContentType],
      },
    );
  }

  @override
  void close({bool force = false}) {}
}

void main() {
  late MemoryStore store;
  late List<KioskEndpoint> probed;
  late Future<void> Function(KioskEndpoint) answer;

  ProviderContainer make({bool allowInsecure = false, KioskEndpoint? initial}) {
    final c = ProviderContainer(
      overrides: [
        endpointStoreProvider.overrideWithValue(store),
        initialKioskEndpointProvider.overrideWithValue(initial),
        allowInsecureServerProvider.overrideWithValue(allowInsecure),
        setupProbeProvider.overrideWithValue((e) {
          probed.add(e);
          return answer(e);
        }),
        deviceLanguagesProvider.overrideWithValue(const ['en']),
      ],
    );
    addTearDown(c.dispose);
    return c;
  }

  SetupController ctl(ProviderContainer c) =>
      c.read(setupControllerProvider.notifier);
  SetupState st(ProviderContainer c) => c.read(setupControllerProvider);

  setUp(() {
    store = MemoryStore();
    probed = [];
    answer = (e) async {};
  });

  test('an address without a scheme is taken as https', () async {
    final c = make();
    await ctl(c).test('shop.example.com', 'MAIN');
    expect(probed.single.origin, 'https://shop.example.com');
    expect(probed.single.branchId, 'MAIN');
  });

  test('a successful test shows the branch code', () async {
    final c = make();
    await ctl(c).test('https://shop.example.com', 'MAIN');
    expect(st(c).branchName, 'MAIN');
    expect(st(c).errorKey, isNull);
    expect(st(c).testing, isFalse);
  });

  test('http is refused when insecure servers are not allowed (release), '
      'and nothing is asked of it', () async {
    final c = make(allowInsecure: false);
    await ctl(c).test('http://shop.example.com', 'MAIN');
    expect(st(c).errorKey, 'serverNotSecure');
    expect(probed, isEmpty);
    expect(st(c).branchName, isNull);
  });

  test('http is accepted when insecure servers are allowed (debug)', () async {
    final c = make(allowInsecure: true);
    await ctl(c).test('http://10.0.2.2:8093', 'MAIN');
    expect(probed.single.origin, 'http://10.0.2.2:8093');
    expect(st(c).branchName, 'MAIN');
  });

  test('an address or branch that does not parse names the problem', () async {
    final c = make();
    await ctl(c).test('', 'MAIN');
    expect(st(c).errorKey, 'serverMissing');
    await ctl(c).test('shop.example.com', '');
    expect(st(c).errorKey, 'branchMissing');
    await ctl(c).test('shop.example.com', 'A B');
    expect(st(c).errorKey, 'branchInvalid');
    expect(probed, isEmpty);
  });

  test('the server\'s own sentence is shown when it refuses', () async {
    final c = make();
    answer = (_) async => throw _dioError('Branch not found.');
    await ctl(c).test('shop.example.com', 'NOPE');
    expect(st(c).serverMessage, 'Branch not found.');
    expect(st(c).branchName, isNull);
  });

  test('no answer at all says the server could not be reached', () async {
    final c = make();
    answer = (_) async => throw _dioError('');
    await ctl(c).test('shop.example.com', 'MAIN');
    expect(st(c).errorKey, 'unreachable');
    expect(st(c).serverMessage, isNull);
  });

  test('nothing is saved before a test has succeeded', () async {
    final c = make();
    expect(await ctl(c).save('shop.example.com', 'MAIN'), isFalse);
    answer = (_) async => throw _dioError('');
    await ctl(c).test('shop.example.com', 'MAIN');
    expect(await ctl(c).save('shop.example.com', 'MAIN'), isFalse);
    expect(store.saves, 0);
    expect(c.read(kioskEndpointProvider), isNull);
  });

  test(
    'nothing is saved when the values differ from the ones tested',
    () async {
      final c = make();
      await ctl(c).test('shop.example.com', 'MAIN');
      expect(await ctl(c).save('shop.example.com', 'OTHER'), isFalse);
      expect(await ctl(c).save('other.example.com', 'MAIN'), isFalse);
      expect(store.saves, 0);
    },
  );

  test('saving stores the endpoint', () async {
    final c = make();
    await ctl(c).test('shop.example.com', 'MAIN');
    expect(await ctl(c).save(' shop.example.com ', 'MAIN'), isTrue);
    final want = KioskEndpoint.parse('https://shop.example.com', 'MAIN');
    expect(store.stored, want);
    expect(c.read(kioskEndpointProvider), want);
  });

  group('the connection test as sent', () {
    test('the setup client carries Origin and the request check header', () {
      final c = make();
      final e = KioskEndpoint.parse('https://shop.example.com:8443', 'B-7');
      final dio = c.read(setupDioFactoryProvider)(e);
      expect(dio.options.baseUrl, 'https://shop.example.com:8443');
      expect(dio.options.headers['Origin'], 'https://shop.example.com:8443');
      expect(dio.options.headers['X-Xchange-Request'], '1');
    });

    test('it probes GET /health/ready', () async {
      final adapter = _Adapter();
      final c = ProviderContainer(
        overrides: [
          endpointStoreProvider.overrideWithValue(store),
          setupDioFactoryProvider.overrideWithValue(
            (e) =>
                buildKioskDio(baseUrl: e.origin, origin: e.origin)
                  ..httpClientAdapter = adapter,
          ),
        ],
      );
      addTearDown(c.dispose);
      final e = KioskEndpoint.parse('https://shop.example.com', 'B-7');
      await c.read(setupProbeProvider)(e);
      expect(adapter.last!.path, '/health/ready');
      expect(adapter.last!.method, 'GET');
    });
  });
}
