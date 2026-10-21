import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../core/endpoint/endpoint_store.dart';
import '../../features/queue_machine/presentation/pages/queue_machine_home_page.dart';
import '../../features/setup/app/setup_providers.dart';
import '../../features/setup/presentation/pages/setup_page.dart';

const setupPath = '/setup';

/// The customer host serves the kiosk at both `/` and `/kiosk`, with the
/// branch in `?branch=`. An installed app with no endpoint goes to setup;
/// the web build has no setup route at all (§8.1).
final appRouterProvider = Provider<GoRouter>(
  (ref) => GoRouter(
    redirect: (context, state) {
      final atSetup = state.matchedLocation == setupPath;
      if (!ref.read(setupAvailableProvider)) return atSetup ? '/' : null;
      final hasEndpoint = ref.read(kioskEndpointProvider) != null;
      if (!hasEndpoint && !atSetup) return setupPath;
      return null;
    },
    // A kiosk switches screens at once: no slide, and never two pages
    // mounted together.
    routes: [
      GoRoute(
        path: '/',
        pageBuilder: (context, state) =>
            const NoTransitionPage(child: QueueMachineHomePage()),
      ),
      GoRoute(
        path: '/kiosk',
        pageBuilder: (context, state) =>
            const NoTransitionPage(child: QueueMachineHomePage()),
      ),
      GoRoute(
        path: setupPath,
        pageBuilder: (context, state) =>
            const NoTransitionPage(child: SetupPage()),
      ),
    ],
    errorBuilder: (context, state) => const QueueMachineHomePage(),
  ),
);
