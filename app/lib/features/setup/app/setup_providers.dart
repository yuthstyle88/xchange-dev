import 'package:dio/dio.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../core/endpoint/endpoint_store.dart';
import '../../../core/endpoint/kiosk_endpoint.dart';
import '../../../core/network/dio_provider.dart';
import '../../kiosk/domain/entities/kiosk_models.dart';
import '../../kiosk/infrastructure/data_sources/kiosk_api.dart';
import '../../kiosk/infrastructure/repositories/kiosk_repository_impl.dart';

/// Whether this app has a setup screen. Off unless start-up turns it on:
/// `main()` sets it to `!kIsWeb`, because the web build takes its endpoint
/// from the page and never shows one (§8.1). A kiosk pumped on its own (a
/// widget test) therefore behaves as it did before setup existed.
final setupAvailableProvider = Provider<bool>((ref) => false);

/// Whether setup accepts a plain http server: in a debug build only, for
/// testing against a local copy. A release build refuses it (§8.1).
final allowInsecureServerProvider = Provider<bool>((ref) => kDebugMode);

/// Asks a server for the rates of one branch, before anything is saved.
typedef SetupProbe = Future<RatesModel> Function(KioskEndpoint endpoint);

/// The HTTP client setup tests a server with: the kiosk's own builder, with
/// the `Origin` header the store would send.
final setupDioFactoryProvider = Provider<Dio Function(KioskEndpoint)>((ref) {
  final store = ref.watch(endpointStoreProvider);
  return (endpoint) => buildKioskDio(
    baseUrl: endpoint.origin,
    origin: store.originHeader(endpoint),
  );
});

final setupProbeProvider = Provider<SetupProbe>((ref) {
  final dioFor = ref.watch(setupDioFactoryProvider);
  return (endpoint) async {
    final dio = dioFor(endpoint);
    try {
      return await KioskRepositoryImpl(
        KioskApi(dio, branchId: endpoint.branchId),
      ).rates();
    } finally {
      dio.close();
    }
  };
});
