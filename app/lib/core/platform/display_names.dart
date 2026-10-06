import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'display_names_io.dart'
    if (dart.library.js_interop) 'display_names_web.dart';

export 'display_names_io.dart'
    if (dart.library.js_interop) 'display_names_web.dart';

abstract class DisplayNames {
  const DisplayNames();

  /// The currency's narrow symbol, or the code when the table has none.
  String currencySymbol(String code);

  /// The currency's name in [lang], or null when not known.
  String? currencyName(String lang, String code);

  /// The region's name in [lang], or null when not known.
  String? regionName(String lang, String region);
}

final displayNamesProvider = Provider<DisplayNames>(
  (ref) => createDisplayNames(),
);
