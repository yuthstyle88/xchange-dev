// Provided with ticket A2 (issue 51, findings 1 and 4 of the A1 review).
// These tests pass on main already: they pin what an installed app depends
// on. Do not edit, delete or weaken them.
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:xchange_kiosk/core/endpoint/endpoint_store.dart';
import 'package:xchange_kiosk/core/endpoint/kiosk_endpoint.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  // The names are written out here on purpose. A test that reads them from
  // the store's own constants stays green when a key is renamed — and then
  // every installed kiosk starts without its server after the update.
  group('the names a device stores its endpoint under', () {
    test('an endpoint written by an earlier version is found', () async {
      SharedPreferences.setMockInitialValues({
        'kiosk.server': 'https://shop.example.com:8443',
        'kiosk.branch': 'B-7',
      });
      expect(
        await createEndpointStore().load(),
        KioskEndpoint.parse('https://shop.example.com:8443', 'B-7'),
      );
    });

    test('saving writes those two names and nothing else', () async {
      SharedPreferences.setMockInitialValues({});
      await createEndpointStore().save(
        KioskEndpoint.parse('https://shop.example.com:8443', 'B-7'),
      );
      final prefs = await SharedPreferences.getInstance();
      expect(prefs.getKeys(), {'kiosk.server', 'kiosk.branch'});
      expect(prefs.getString('kiosk.server'), 'https://shop.example.com:8443');
      expect(prefs.getString('kiosk.branch'), 'B-7');
    });

    test('clearing removes both', () async {
      SharedPreferences.setMockInitialValues({
        'kiosk.server': 'https://shop.example.com:8443',
        'kiosk.branch': 'B-7',
        'something.else': 'kept',
      });
      await createEndpointStore().clear();
      final prefs = await SharedPreferences.getInstance();
      expect(prefs.getKeys(), {'something.else'});
    });
  });

  test('createEndpointStore is the only factory of an EndpointStore', () {
    final factories = <String>[];
    var files = 0;
    for (final f in Directory('lib').listSync(recursive: true)) {
      if (f is! File || !f.path.endsWith('.dart')) {
        continue;
      }
      files++;
      for (final m in RegExp(
        r'^EndpointStore\s+(\w+)\s*\(',
        multiLine: true,
      ).allMatches(f.readAsStringSync())) {
        factories.add(m[1]!);
      }
    }
    expect(files, greaterThan(20));
    // One per target, picked by the conditional import.
    expect(factories, ['createEndpointStore', 'createEndpointStore']);
  });
}
