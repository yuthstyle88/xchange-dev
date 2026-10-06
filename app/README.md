# XCHANGE customer kiosk

The four customer-facing screens of the XCHANGE counter, as Flutter web:
choose and see rates → quote with countdown → waiting in the queue → receipt.

Design: `../docs/design/xchange-v3.md` §8. Layout and stack follow `108heros-flutter`.

## Run it

The server refuses any POST whose `Origin` is not its own host, so the kiosk
must be served from the same origin as `/api/kiosk`. `tool/dev_server.mjs`
does that for development:

```bash
flutter build web --release
node tool/dev_server.mjs 8094 http://127.0.0.1:8093   # kiosk on :8094, XCHANGE server on :8093
```

Open `http://localhost:8094/kiosk?branch=MAIN`.

## Check it

```bash
dart format --output=none --set-exit-if-changed lib test
flutter analyze
flutter test
```

`assets/l10n/kiosk.json` and `assets/img/` were exported from the v2 kiosk
(`kiosk-i18n.js`, `kiosk-language-extras.js`, `kiosk.html`) and are not edited by hand.
