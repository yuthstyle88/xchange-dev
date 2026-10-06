// Provided with ticket A1. These tests are the specification: make them pass,
// do not edit, delete or weaken them.
//
// On the Dart VM the conditional import picks the device implementation, the
// one an installed app uses. The page implementation runs only in a browser.
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:xchange_kiosk/core/endpoint/endpoint_store.dart';
import 'package:xchange_kiosk/core/endpoint/kiosk_endpoint.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  late ProviderContainer container;
  late EndpointStore store;
  final shop = KioskEndpoint.parse('https://shop.example.com', 'B-7');

  setUp(() {
    SharedPreferences.setMockInitialValues({});
    container = ProviderContainer();
    store = container.read(endpointStoreProvider);
  });
  tearDown(() => container.dispose());

  test('a device that was never set up has no endpoint', () async {
    expect(await store.load(), isNull);
  });

  test('what was saved is what a later start loads', () async {
    await store.save(shop);
    // A fresh container stands for the app being started again.
    final again = ProviderContainer();
    addTearDown(again.dispose);
    expect(await again.read(endpointStoreProvider).load(), shop);
  });

  test('saving again replaces the endpoint', () async {
    await store.save(shop);
    final other = KioskEndpoint.parse('https://other.example.com', 'MAIN');
    await store.save(other);
    expect(await store.load(), other);
  });

  test('clearing forgets it', () async {
    await store.save(shop);
    await store.clear();
    expect(await store.load(), isNull);
  });

  test('half a record, or one that no longer parses, is no endpoint', () async {
    for (final values in <Map<String, Object>>[
      {'kiosk.server': 'https://shop.example.com'},
      {'kiosk.branch': 'B-7'},
      {'kiosk.server': 'not a host', 'kiosk.branch': 'B-7'},
      {'kiosk.server': 'https://shop.example.com', 'kiosk.branch': ''},
    ]) {
      SharedPreferences.setMockInitialValues(values);
      final fresh = ProviderContainer();
      addTearDown(fresh.dispose);
      expect(
        await fresh.read(endpointStoreProvider).load(),
        isNull,
        reason: '$values',
      );
    }
  });

  test('an http endpoint saved by a debug build still loads', () async {
    final local = KioskEndpoint.parse(
      'http://192.168.1.75:8095',
      'MAIN',
      allowInsecure: true,
    );
    await store.save(local);
    expect(await store.load(), local);
  });

  test('an installed app names the server as its own origin', () {
    // The server refuses a POST whose Origin is not its own host. A browser
    // sends the header by itself; an app has to.
    expect(store.originHeader(shop), 'https://shop.example.com');
  });

  group('the endpoint the app is using', () {
    test('starts as what main loaded', () {
      final seeded = ProviderContainer(
        overrides: [initialKioskEndpointProvider.overrideWithValue(shop)],
      );
      addTearDown(seeded.dispose);
      expect(seeded.read(kioskEndpointProvider), shop);
    });

    test('is absent when nothing was loaded', () {
      expect(container.read(kioskEndpointProvider), isNull);
    });

    test('can be set and cleared, and listeners hear both', () {
      final heard = <KioskEndpoint?>[];
      container.listen(kioskEndpointProvider, (_, next) => heard.add(next));
      container.read(kioskEndpointProvider.notifier).set(shop);
      expect(container.read(kioskEndpointProvider), shop);
      container.read(kioskEndpointProvider.notifier).clear();
      expect(container.read(kioskEndpointProvider), isNull);
      expect(heard, [shop, null]);
    });
  });
}
