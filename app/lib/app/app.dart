import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../features/kiosk/app/kiosk_providers.dart';
import '../shared/theme/theme.dart';
import 'router/app_router.dart';

class KioskApp extends ConsumerWidget {
  const KioskApp({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final languageCode = ref.watch(
      kioskControllerProvider.select((s) => s.languageCode),
    );

    return MaterialApp.router(
      title: 'XCHANGE · Customer Touch Screen',
      debugShowCheckedModeBanner: false,
      theme: kioskTheme(languageCode),
      routerConfig: ref.watch(appRouterProvider),
    );
  }
}
