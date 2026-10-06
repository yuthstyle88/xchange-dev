import 'dart:js_interop';

import 'display_names.dart';

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

final _symbols = <String, String>{};
final _names = <String, _DisplayNames?>{};

class BrowserDisplayNames implements DisplayNames {
  const BrowserDisplayNames();

  @override
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

  @override
  String? currencyName(String lang, String code) =>
      _displayName(lang, 'currency', code);

  @override
  String? regionName(String lang, String region) =>
      _displayName(lang, 'region', region);
}

DisplayNames createDisplayNames() => const BrowserDisplayNames();

Future<DisplayNames> loadDisplayNames() =>
    Future.value(const BrowserDisplayNames());
