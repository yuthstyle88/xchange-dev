List<String> deviceLanguages() => const ['en'];

/// The currency's narrow symbol, or the code when the browser has none.
String currencySymbol(String code) => code;

/// The currency's name in [lang], or null when the browser cannot say.
String? currencyDisplayName(String lang, String code) => null;

/// The region's name in [lang], or null when the browser cannot say.
String? regionDisplayName(String lang, String regionCode) => null;

void saveDataUrl(String dataUrl, String filename) {}

void openExternal(String url) {}
