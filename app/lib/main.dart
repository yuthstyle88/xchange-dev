import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_riverpod/misc.dart';
import 'package:flutter_web_plugins/url_strategy.dart';

import 'app/app.dart';
import 'core/endpoint/endpoint_store.dart';
import 'core/platform/display_names.dart';
import 'features/kiosk/app/kiosk_providers.dart';
import 'l10n/kiosk_l10n.dart';

Future<ProviderContainer> bootstrap({
  required KioskL10n l10n,
  required EndpointStore store,
  List<Override> overrides = const [],
}) async {
  final endpoint = await store.load();
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
    overrides: [displayNamesProvider.overrideWithValue(names)],
  );
  runApp(
    UncontrolledProviderScope(container: container, child: const KioskApp()),
  );
}
