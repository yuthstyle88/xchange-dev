/// What the app needs from the browser. Tests run on the VM, where the stub
/// answers; the web build gets the real thing.
library;

export 'browser_stub.dart' if (dart.library.js_interop) 'browser_web.dart';
