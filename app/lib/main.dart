import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_web_plugins/url_strategy.dart';

import 'app/app.dart';
import 'features/kiosk/app/kiosk_providers.dart';
import 'l10n/kiosk_l10n.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  // The kiosk lives at a real path with `?branch=`, not behind a `#`.
  usePathUrlStrategy();
  // Loaded before the first frame: the controller reads the texts
  // synchronously to pick the device's language.
  final l10n = await KioskL10n.load();
  final container = ProviderContainer(
    overrides: [kioskL10nProvider.overrideWithValue(l10n)],
  );
  runApp(
    UncontrolledProviderScope(container: container, child: const KioskApp()),
  );
  container.read(kioskControllerProvider.notifier).start();
}
