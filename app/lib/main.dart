import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_riverpod/misc.dart';
import 'package:flutter_web_plugins/url_strategy.dart';

import 'app/app.dart';
import 'core/endpoint/endpoint_store.dart';
import 'core/endpoint/kiosk_endpoint.dart';
import 'core/platform/display_names.dart';
import 'features/kiosk/app/kiosk_providers.dart';
import 'features/setup/app/setup_providers.dart';
import 'l10n/kiosk_l10n.dart';

Future<ProviderContainer> bootstrap({
  required KioskL10n l10n,
  required EndpointStore store,
  List<Override> overrides = const [],
}) async {
  // A refused endpoint (a `?branch=` the kiosk cannot use) must not stop the
  // app from starting: the kiosk shows its error line and asks nothing.
  KioskEndpoint? endpoint;
  var refused = false;
  try {
    endpoint = await store.load();
  } on EndpointFormatException {
    refused = true;
  }
  final container = ProviderContainer(
    overrides: [
      kioskL10nProvider.overrideWithValue(l10n),
      endpointStoreProvider.overrideWithValue(store),
      initialKioskEndpointProvider.overrideWithValue(endpoint),
      ...overrides,
    ],
  );

  if (endpoint != null) {
    container.read(kioskControllerProvider.notifier).start();
  } else if (refused) {
    container.read(kioskControllerProvider.notifier).reportError('errGeneric');
  }

  return container;
}

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  // The kiosk lives at a real path with `?branch=`, not behind a `#`.
  usePathUrlStrategy();
  // Loaded before the first frame: the controller reads the texts
  // synchronously to pick the device's language.
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
