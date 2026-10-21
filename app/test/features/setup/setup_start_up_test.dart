// #174 (A3): start-up with an endpoint the app refuses, the IPv6 origin,
// and the setup texts. Written before the code.
import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/core/endpoint/endpoint_store.dart';
import 'package:xchange_kiosk/core/endpoint/kiosk_endpoint.dart';
import 'package:xchange_kiosk/l10n/kiosk_l10n.dart';
import 'package:xchange_kiosk/main.dart' as app;

KioskL10n texts() => KioskL10n.fromJson(
  jsonDecode(File('assets/l10n/kiosk.json').readAsStringSync())
      as Map<String, dynamic>,
);

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
  test('a refused endpoint leaves the stored endpoint null', () async {
    final c = await app.bootstrap(l10n: texts(), store: RefusingStore());
    addTearDown(c.dispose);
    await Future<void>.delayed(Duration.zero);

    expect(c.read(kioskEndpointProvider), isNull);
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
