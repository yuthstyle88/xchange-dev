import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_riverpod/legacy.dart';

import '../shared/theme/theme.dart';
import 'router/app_router.dart';

final appLanguageProvider = StateProvider<String>((ref) => 'th');

class KioskApp extends ConsumerWidget {
  const KioskApp({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final languageCode = ref.watch(appLanguageProvider);

    return MaterialApp.router(
      title: 'XCHANGE · Customer Touch Screen',
      debugShowCheckedModeBanner: false,
      theme: kioskTheme(languageCode),
      routerConfig: ref.watch(appRouterProvider),
    );
  }
}
