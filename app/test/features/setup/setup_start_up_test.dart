// #174 (A3): start-up with an endpoint the kiosk refuses, the IPv6 origin,
// and the setup texts. Written before the code.
//
// Mutations that must turn this red:
// - let bootstrap rethrow EndpointFormatException -> a refused branch starts
// - start the controller on a refused endpoint    -> … nothing is asked
// - drop the brackets from an IPv6 host           -> IPv6 keeps its brackets
// - leave a key out of one language               -> every key in en and th
import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/core/endpoint/endpoint_store.dart';
import 'package:xchange_kiosk/core/endpoint/kiosk_endpoint.dart';
import 'package:xchange_kiosk/features/kiosk/app/kiosk_providers.dart';
import 'package:xchange_kiosk/main.dart' as app;

import '../kiosk/kiosk_controller_test.dart' show FakeRepository, l10n;

/// What `PageEndpointStore` does with `?branch=A%20B`: it throws.
class RefusingStore implements EndpointStore {
  @override
  Future<KioskEndpoint?> load() async =>
      throw const EndpointFormatException(EndpointProblem.branchInvalid);

  @override
  Future<void> save(KioskEndpoint endpoint) async {}

  @override
  Future<void> clear() async {}

  @override
  String? originHeader(KioskEndpoint endpoint) => null;
}

void main() {
  test('a refused endpoint starts the kiosk with its error line, and nothing '
      'is asked of any server', () async {
    final repo = FakeRepository();
    final c = await app.bootstrap(
      l10n: l10n(),
      store: RefusingStore(),
      overrides: [
        kioskRepositoryProvider.overrideWithValue(repo),
        deviceLanguagesProvider.overrideWithValue(const ['en']),
      ],
    );
    addTearDown(c.dispose);
    await Future<void>.delayed(Duration.zero);

    expect(c.read(kioskEndpointProvider), isNull);
    expect(c.read(kioskControllerProvider).error, 'generic-en');
    expect(repo.calls, isEmpty);
  });

  group('IPv6', () {
    test('an IPv6 literal keeps its brackets in the origin', () {
      final e = KioskEndpoint.parse('https://[::1]:8443', 'MAIN');
      expect(e.origin, 'https://[::1]:8443');
    });

    test('… also on the default port', () {
      final e = KioskEndpoint.parse('https://[2001:db8::7]', 'MAIN');
      expect(e.origin, 'https://[2001:db8::7]');
    });

    test('a name is unchanged', () {
      final e = KioskEndpoint.parse('https://shop.example.com:8443', 'MAIN');
      expect(e.origin, 'https://shop.example.com:8443');
    });
  });

  group('setup texts', () {
    Map<String, dynamic> read() =>
        jsonDecode(File('assets/l10n/setup.json').readAsStringSync())
            as Map<String, dynamic>;
    const keys = [
      'title',
      'server',
      'branch',
      'test',
      'testing',
      'save',
      'back',
      'connected',
      'serverMissing',
      'serverInvalid',
      'serverNotSecure',
      'branchMissing',
      'branchInvalid',
      'unreachable',
      'notTested',
    ];

    test('English and Thai only, each with every key', () {
      final json = read();
      expect(json.keys.toSet(), {'en', 'th'});
      for (final lang in ['en', 'th']) {
        final t = json[lang] as Map<String, dynamic>;
        for (final k in keys) {
          expect((t[k] as String?)?.trim(), isNotEmpty, reason: '$lang.$k');
        }
      }
    });

    test('the http refusal says the server must use https', () {
      final json = read();
      expect((json['en'] as Map)['serverNotSecure'], contains('https'));
      expect((json['th'] as Map)['serverNotSecure'], contains('https'));
    });
  });
}
