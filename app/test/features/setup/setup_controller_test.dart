// #174 (A3): what the setup screen does with a server address and a branch
// code. Written before the code.
//
// Mutations that must turn this red:
// - drop the "https://" default                 -> no scheme assumes https
// - accept http when insecure is not allowed    -> http is refused in release
// - save without a successful test              -> nothing is saved until …
// - save values other than the tested ones      -> … the same values …
// - set the endpoint but keep the old controller -> saving starts the kiosk on
//                                                   the new endpoint
// - show "unreachable" for a server's sentence  -> the server's own sentence
import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:dio/dio.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/core/endpoint/endpoint_store.dart';
import 'package:xchange_kiosk/core/endpoint/kiosk_endpoint.dart';
import 'package:xchange_kiosk/core/network/dio_provider.dart';
import 'package:xchange_kiosk/features/kiosk/app/kiosk_providers.dart';
import 'package:xchange_kiosk/features/kiosk/domain/entities/kiosk_models.dart';
import 'package:xchange_kiosk/features/kiosk/domain/kiosk_failure.dart';
import 'package:xchange_kiosk/features/kiosk/domain/repository/kiosk_repository.dart';
import 'package:xchange_kiosk/features/setup/app/setup_providers.dart';
import 'package:xchange_kiosk/features/setup/presentation/controllers/setup_controller.dart';

import '../kiosk/kiosk_controller_test.dart' show l10n;

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

RatesModel ratesFor(String branchId, String name) => RatesModel.fromJson({
  'branchId': branchId,
  'branchName': name,
  'open': true,
  'rates': <Object>[],
});

/// A repository that says which branch it was built for, so a test can see
/// which endpoint the kiosk controller is talking to.
class BranchRepo implements KioskRepository {
  BranchRepo(this.branch, this.log);
  final String branch;
  final List<String> log;

  @override
  Future<RatesModel> rates() async {
    log.add(branch);
    return ratesFor(branch, 'name-$branch');
  }

  @override
  dynamic noSuchMethod(Invocation invocation) => throw UnimplementedError();
}

class _Adapter implements HttpClientAdapter {
  RequestOptions? last;
  String? sent;

  @override
  Future<ResponseBody> fetch(
    RequestOptions options,
    Stream<Uint8List>? requestStream,
    Future<void>? cancelFuture,
  ) async {
    last = options;
    sent = requestStream == null
        ? null
        : utf8.decode(await requestStream.expand((c) => c).toList());
    return ResponseBody.fromString(
      jsonEncode({
        'branchId': 'B-7',
        'branchName': 'Siam',
        'open': true,
        'rates': <Object>[],
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

void main() {
  late MemoryStore store;
  late List<KioskEndpoint> probed;
  late Future<RatesModel> Function(KioskEndpoint) answer;
  late List<String> repoLog;

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
        kioskL10nProvider.overrideWithValue(l10n()),
        deviceLanguagesProvider.overrideWithValue(const ['en']),
        kioskRepositoryProvider.overrideWith(
          (ref) => BranchRepo(ref.watch(kioskBranchProvider), repoLog),
        ),
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
    repoLog = [];
    answer = (e) async => ratesFor(e.branchId, 'Siam');
  });

  test('an address without a scheme is taken as https', () async {
    final c = make();
    await ctl(c).test('shop.example.com', 'MAIN');
    expect(probed.single.origin, 'https://shop.example.com');
    expect(probed.single.branchId, 'MAIN');
  });

  test(
    'a successful test shows the branch name the server answered with',
    () async {
      final c = make();
      await ctl(c).test('https://shop.example.com', 'MAIN');
      expect(st(c).branchName, 'Siam');
      expect(st(c).errorKey, isNull);
      expect(st(c).testing, isFalse);
    },
  );

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
    expect(st(c).branchName, 'Siam');
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
    answer = (_) async => throw const KioskFailure('Branch not found.');
    await ctl(c).test('shop.example.com', 'NOPE');
    expect(st(c).serverMessage, 'Branch not found.');
    expect(st(c).branchName, isNull);
  });

  test('no answer at all says the server could not be reached', () async {
    final c = make();
    answer = (_) async => throw const KioskFailure('');
    await ctl(c).test('shop.example.com', 'MAIN');
    expect(st(c).errorKey, 'unreachable');
    expect(st(c).serverMessage, isNull);
  });

  test('nothing is saved before a test has succeeded', () async {
    final c = make();
    expect(await ctl(c).save('shop.example.com', 'MAIN'), isFalse);
    answer = (_) async => throw const KioskFailure('');
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

  test('saving stores the endpoint and starts the kiosk on it', () async {
    final c = make();
    await ctl(c).test('shop.example.com', 'MAIN');
    expect(await ctl(c).save(' shop.example.com ', 'MAIN'), isTrue);
    final want = KioskEndpoint.parse('https://shop.example.com', 'MAIN');
    expect(store.stored, want);
    expect(c.read(kioskEndpointProvider), want);
    await Future<void>.delayed(Duration.zero);
    expect(repoLog, ['MAIN']);
    expect(c.read(kioskControllerProvider).model?.branchName, 'name-MAIN');
  });

  test('saving a new endpoint later moves the running kiosk to it', () async {
    final old = KioskEndpoint.parse('https://old.example.com', 'OLD');
    store.stored = old;
    final c = make(initial: old);
    await c.read(kioskControllerProvider.notifier).start();
    expect(repoLog, ['OLD']);

    await ctl(c).test('new.example.com', 'NEW');
    expect(await ctl(c).save('new.example.com', 'NEW'), isTrue);
    await Future<void>.delayed(Duration.zero);

    expect(repoLog.last, 'NEW');
    expect(c.read(kioskControllerProvider).model?.branchName, 'name-NEW');
    // and it keeps talking to the new server afterwards
    await c.read(kioskControllerProvider.notifier).reset();
    expect(repoLog.last, 'NEW');
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

    test('it asks for the rates of that branch', () async {
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
      final rates = await c.read(setupProbeProvider)(e);
      expect(rates.branchName, 'Siam');
      expect(adapter.last!.path, '/api/kiosk');
      final body = jsonDecode(adapter.sent!) as Map<String, dynamic>;
      expect(body['action'], 'rates');
      expect((body['payload'] as Map)['branchId'], 'B-7');
    });
  });
}
