import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../shared/theme/theme.dart';
import 'router/app_router.dart';

class KioskApp extends ConsumerWidget {
  const KioskApp({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) => MaterialApp.router(
    title: 'XCHANGE · Customer Touch Screen',
    debugShowCheckedModeBanner: false,
    theme: kioskTheme(),
    routerConfig: ref.watch(appRouterProvider),
  );
}
