import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_riverpod/misc.dart';
import 'package:flutter_web_plugins/url_strategy.dart';

import 'app/app.dart';
import 'core/endpoint/endpoint_store.dart';
import 'core/endpoint/kiosk_endpoint.dart';
import 'core/platform/display_names.dart';
import 'features/setup/app/setup_providers.dart';
import 'l10n/kiosk_l10n.dart';

final kioskL10nProvider = Provider<KioskL10n>(
  (ref) => throw StateError('kioskL10nProvider must be overridden in main'),
);

/// The branch the kiosk is serving: the endpoint's branchId. Throws StateError when there is no endpoint.
final kioskBranchProvider = Provider<String>((ref) {
  final endpoint = ref.watch(kioskEndpointProvider);
  if (endpoint == null) {
    throw StateError('No kiosk endpoint set');
  }
  return endpoint.branchId;
});

Future<ProviderContainer> bootstrap({
  required KioskL10n l10n,
  required EndpointStore store,
  List<Override> overrides = const [],
}) async {
  // A refused endpoint (a `?branch=` the kiosk cannot use) must not stop the
  // app from starting: the kiosk shows its error line and asks nothing.
  KioskEndpoint? endpoint;
  try {
    endpoint = await store.load();
  } on EndpointFormatException {
    // Refused endpoint
  }
  final container = ProviderContainer(
    overrides: [
      kioskL10nProvider.overrideWithValue(l10n),
      endpointStoreProvider.overrideWithValue(store),
      initialKioskEndpointProvider.overrideWithValue(endpoint),
      ...overrides,
    ],
  );

  return container;
}

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  // The kiosk lives at a real path with `?branch=`, not behind a `#`.
  usePathUrlStrategy();
  // Loaded before the first frame.
  final l10n = await KioskL10n.load();
  final store = createEndpointStore();
  final names = await loadDisplayNames();
  final container = await bootstrap(
    l10n: l10n,
    store: store,
    overrides: [
      displayNamesProvider.overrideWithValue(names),
      setupAvailableProvider.overrideWithValue(!kIsWeb),
    ],
  );
  runApp(
    UncontrolledProviderScope(container: container, child: const KioskApp()),
  );
}
