import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../core/endpoint/endpoint_store.dart';
import '../../../core/network/dio_provider.dart';
import '../../../core/platform/browser.dart' as browser;
import '../../../l10n/kiosk_l10n.dart';
import '../domain/repository/kiosk_repository.dart';
import '../infrastructure/data_sources/kiosk_api.dart';
import '../infrastructure/repositories/kiosk_repository_impl.dart';
import '../presentation/controllers/kiosk_controller.dart';

/// The branch the kiosk is serving: the endpoint's branchId. Throws StateError when there is no endpoint.
final kioskBranchProvider = Provider<String>((ref) {
  final endpoint = ref.watch(kioskEndpointProvider);
  if (endpoint == null) {
    throw StateError('No kiosk endpoint set');
  }
  return endpoint.branchId;
});

/// The device's languages, most wanted first.
final deviceLanguagesProvider = Provider<List<String>>(
  (ref) => browser.deviceLanguages(),
);

/// The kiosk's texts. Loaded from the asset in `main` before the first frame
/// and supplied by override, because everything that reads it is synchronous.
final kioskL10nProvider = Provider<KioskL10n>(
  (ref) => throw StateError('kioskL10nProvider must be overridden in main'),
);

final kioskApiProvider = Provider<KioskApi>(
  (ref) => KioskApi(
    ref.watch(dioProvider),
    branchId: ref.watch(kioskBranchProvider),
  ),
);

final kioskRepositoryProvider = Provider<KioskRepository>(
  (ref) => KioskRepositoryImpl(ref.watch(kioskApiProvider)),
);

final kioskControllerProvider = NotifierProvider<KioskController, KioskState>(
  KioskController.new,
);

/// The texts in the language the kiosk is showing right now.
final kioskTextsProvider = Provider<KioskTexts>(
  (ref) => ref
      .watch(kioskL10nProvider)
      .of(ref.watch(kioskControllerProvider.select((s) => s.languageCode))),
);
