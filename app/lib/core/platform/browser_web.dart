import 'dart:js_interop';

import 'package:web/web.dart' as web;

@JS('Intl.DisplayNames')
extension type _DisplayNames._(JSObject _) implements JSObject {
  external _DisplayNames(
    JSArray<JSString> locales,
    _DisplayNamesOptions options,
  );
  external JSString? of(JSString code);
}

extension type _DisplayNamesOptions._(JSObject _) implements JSObject {
  external factory _DisplayNamesOptions({JSString type});
}

@JS('Intl.NumberFormat')
extension type _NumberFormat._(JSObject _) implements JSObject {
  external _NumberFormat(JSString locale, _NumberFormatOptions options);
  external JSArray<_Part> formatToParts(JSNumber n);
}

extension type _NumberFormatOptions._(JSObject _) implements JSObject {
  external factory _NumberFormatOptions({
    JSString style,
    JSString currency,
    JSString currencyDisplay,
  });
}

extension type _Part._(JSObject _) implements JSObject {
  external JSString get type;
  external JSString get value;
}

List<String> deviceLanguages() {
  final list = web.window.navigator.languages.toDart
      .map((l) => l.toDart)
      .toList();
  return list.isEmpty ? [web.window.navigator.language] : list;
}

final _symbols = <String, String>{};

String currencySymbol(String code) => _symbols.putIfAbsent(code, () {
  try {
    final parts = _NumberFormat(
      'en-US'.toJS,
      _NumberFormatOptions(
        style: 'currency'.toJS,
        currency: code.toJS,
        currencyDisplay: 'narrowSymbol'.toJS,
      ),
    ).formatToParts(0.toJS).toDart;
    for (final p in parts) {
      if (p.type.toDart == 'currency') return p.value.toDart;
    }
  } catch (_) {
    // Unknown code or an old browser: the code itself is the label.
  }
  return code;
});

final _names = <String, _DisplayNames?>{};

String? _displayName(String lang, String type, String code) {
  final names = _names.putIfAbsent('$lang|$type', () {
    try {
      return _DisplayNames(
        [lang.toJS, 'en'.toJS].toJS,
        _DisplayNamesOptions(type: type.toJS),
      );
    } catch (_) {
      return null;
    }
  });
  try {
    final name = names?.of(code.toJS)?.toDart;
    return name == null || name == code ? null : name;
  } catch (_) {
    return null;
  }
}

String? currencyDisplayName(String lang, String code) =>
    _displayName(lang, 'currency', code);

String? regionDisplayName(String lang, String regionCode) =>
    _displayName(lang, 'region', regionCode);

void saveDataUrl(String dataUrl, String filename) {
  final a = web.document.createElement('a') as web.HTMLAnchorElement
    ..href = dataUrl
    ..download = filename;
  web.document.body!.append(a);
  a.click();
  a.remove();
}

void openExternal(String url) => web.window.open(url, '_blank', 'noopener');
