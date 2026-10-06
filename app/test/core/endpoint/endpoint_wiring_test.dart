// Provided with ticket A1. These tests are the specification: make them pass,
// do not edit, delete or weaken them.
import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:xchange_kiosk/core/endpoint/endpoint_store.dart';
import 'package:xchange_kiosk/core/endpoint/kiosk_endpoint.dart';
import 'package:xchange_kiosk/core/network/dio_provider.dart';
import 'package:xchange_kiosk/features/kiosk/app/kiosk_providers.dart';
import 'package:xchange_kiosk/features/kiosk/domain/entities/kiosk_models.dart';
import 'package:xchange_kiosk/features/kiosk/domain/repository/kiosk_repository.dart';
import 'package:xchange_kiosk/l10n/kiosk_l10n.dart';
import 'package:xchange_kiosk/main.dart' as app;

/// A store that answers what the test gives it and remembers nothing else.
class _Store implements EndpointStore {
  _Store(this.stored, {this.origin});
  KioskEndpoint? stored;
  final String? origin;
  int loads = 0;

  @override
  Future<KioskEndpoint?> load() async {
    loads++;
    return stored;
  }

  @override
  Future<void> save(KioskEndpoint endpoint) async => stored = endpoint;

  @override
  Future<void> clear() async => stored = null;

  @override
  String? originHeader(KioskEndpoint endpoint) => origin;
}

/// Counts how often the kiosk asks for the rates and never answers.
class _Repo implements KioskRepository {
  int ratesCalls = 0;

  @override
  Future<RatesModel> rates() {
    ratesCalls++;
    return Completer<RatesModel>().future;
  }

  @override
  dynamic noSuchMethod(Invocation invocation) => throw UnimplementedError();
}

KioskL10n texts() => KioskL10n.fromJson(
  jsonDecode(File('assets/l10n/kiosk.json').readAsStringSync())
      as Map<String, dynamic>,
);

/// Riverpod 3 hands a provider's error to the reader wrapped in its own
/// exception type; the rule is about the cause.
final Matcher _throwsStateError = throwsA(
  predicate<Object>(
    (e) => e is StateError || (e as dynamic).exception is StateError,
    'a StateError, bare or as the cause of a provider failure',
  ),
);

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final shop = KioskEndpoint.parse('https://shop.example.com:8443', 'B-7');

  group('the network layer reads the endpoint', () {
    ProviderContainer containerWith(KioskEndpoint? endpoint, _Store store) {
      final c = ProviderContainer(
        overrides: [
          initialKioskEndpointProvider.overrideWithValue(endpoint),
          endpointStoreProvider.overrideWithValue(store),
        ],
      );
      addTearDown(c.dispose);
      return c;
    }

    test('the base address is the server origin and the branch is its own', () {
      final c = containerWith(shop, _Store(shop));
      expect(c.read(apiBaseUrlProvider), 'https://shop.example.com:8443');
      expect(
        c.read(dioProvider).options.baseUrl,
        'https://shop.example.com:8443',
      );
      expect(c.read(kioskBranchProvider), 'B-7');
    });

    test('the request check header is always sent', () {
      final c = containerWith(shop, _Store(shop));
      expect(c.read(dioProvider).options.headers['X-Xchange-Request'], '1');
    });

    test('Origin is sent when the store supplies one', () {
      final c = containerWith(
        shop,
        _Store(shop, origin: 'https://shop.example.com:8443'),
      );
      expect(
        c.read(dioProvider).options.headers['Origin'],
        'https://shop.example.com:8443',
      );
    });

    test('Origin is not set when the store supplies none (a browser sets it '
        'itself and refuses a script that tries)', () {
      final c = containerWith(shop, _Store(shop));
      expect(
        c.read(dioProvider).options.headers.keys.map((k) => k.toLowerCase()),
        isNot(contains('origin')),
      );
    });

    test('changing the endpoint changes where the next request goes', () {
      final c = containerWith(shop, _Store(shop));
      expect(c.read(apiBaseUrlProvider), 'https://shop.example.com:8443');
      c
          .read(kioskEndpointProvider.notifier)
          .set(KioskEndpoint.parse('https://other.example.com', 'MAIN'));
      expect(c.read(apiBaseUrlProvider), 'https://other.example.com');
      expect(c.read(dioProvider).options.baseUrl, 'https://other.example.com');
      expect(c.read(kioskBranchProvider), 'MAIN');
    });

    test('asking for the server with no endpoint is a programming error, not '
        'a request to localhost', () {
      final c = containerWith(null, _Store(null));
      expect(() => c.read(apiBaseUrlProvider), _throwsStateError);
      expect(() => c.read(kioskBranchProvider), _throwsStateError);
    });
  });

  group('start-up', () {
    setUp(() => SharedPreferences.setMockInitialValues({}));

    test('with nothing stored the kiosk is not started and nothing is asked '
        'of any server', () async {
      final store = _Store(null);
      final repo = _Repo();
      final c = await app.bootstrap(
        l10n: texts(),
        store: store,
        overrides: [kioskRepositoryProvider.overrideWithValue(repo)],
      );
      addTearDown(c.dispose);
      await Future<void>.delayed(Duration.zero);

      expect(store.loads, 1);
      expect(c.read(kioskEndpointProvider), isNull);
      expect(repo.ratesCalls, 0);
    });

    test('with an endpoint stored the kiosk starts on it', () async {
      final store = _Store(shop);
      final repo = _Repo();
      final c = await app.bootstrap(
        l10n: texts(),
        store: store,
        overrides: [kioskRepositoryProvider.overrideWithValue(repo)],
      );
      addTearDown(c.dispose);
      await Future<void>.delayed(Duration.zero);

      expect(store.loads, 1);
      expect(c.read(kioskEndpointProvider), shop);
      expect(c.read(apiBaseUrlProvider), 'https://shop.example.com:8443');
      expect(c.read(kioskBranchProvider), 'B-7');
      expect(repo.ratesCalls, 1);
    });

    test('the container uses the store it was given', () async {
      final store = _Store(shop);
      final c = await app.bootstrap(
        l10n: texts(),
        store: store,
        overrides: [kioskRepositoryProvider.overrideWithValue(_Repo())],
      );
      addTearDown(c.dispose);
      expect(c.read(endpointStoreProvider), same(store));
    });
  });
}
