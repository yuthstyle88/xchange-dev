import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'endpoint_store_io.dart'
    if (dart.library.js_interop) 'endpoint_store_web.dart';
import 'kiosk_endpoint.dart';

export 'endpoint_store_io.dart'
    if (dart.library.js_interop) 'endpoint_store_web.dart';

abstract class EndpointStore {
  Future<KioskEndpoint?> load();
  Future<void> save(KioskEndpoint endpoint);
  Future<void> clear();
  String? originHeader(KioskEndpoint endpoint);
}

final endpointStoreProvider = Provider<EndpointStore>((ref) {
  return createEndpointStore();
});

final initialKioskEndpointProvider = Provider<KioskEndpoint?>((ref) => null);

class KioskEndpointNotifier extends Notifier<KioskEndpoint?> {
  @override
  KioskEndpoint? build() => ref.watch(initialKioskEndpointProvider);

  void set(KioskEndpoint endpoint) {
    state = endpoint;
  }

  void clear() {
    state = null;
  }
}

final kioskEndpointProvider =
    NotifierProvider<KioskEndpointNotifier, KioskEndpoint?>(
      KioskEndpointNotifier.new,
    );
