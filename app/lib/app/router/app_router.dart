import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../features/kiosk/presentation/pages/kiosk_page.dart';

/// The customer host serves the kiosk at both `/` and `/kiosk`, with the
/// branch in `?branch=`.
final appRouterProvider = Provider<GoRouter>(
  (ref) => GoRouter(
    routes: [
      GoRoute(path: '/', builder: (context, state) => const KioskPage()),
      GoRoute(path: '/kiosk', builder: (context, state) => const KioskPage()),
    ],
    errorBuilder: (context, state) => const KioskPage(),
  ),
);
