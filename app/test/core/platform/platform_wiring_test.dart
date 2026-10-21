// Provided with ticket A2. These tests are the specification: make them pass,
// do not edit, delete or weaken them.
//
// The app reaches the device and the names only through two providers, so
// a test (and the other target) can put its own implementation there.
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/core/platform/device_actions.dart';
import 'package:xchange_kiosk/core/platform/display_names.dart';
import 'package:xchange_kiosk/features/setup/app/setup_providers.dart';

/// Device actions that record what the pages ask.
class _Device extends DeviceActions {
  _Device({this.languageList = const ['en']});
  final List<String> languageList;
  final saved = <(String, String)>[];
  final opened = <String>[];

  @override
  List<String> languages() => languageList;

  @override
  Future<void> saveImage(String dataUrl, String filename) async {
    saved.add((dataUrl, filename));
  }

  @override
  Future<void> openHttps(Uri url) async {
    opened.add('$url');
  }
}

void main() {
  group('providers', () {
    test('on a device the defaults are the device implementations', () {
      final c = ProviderContainer();
      addTearDown(c.dispose);
      expect(c.read(deviceActionsProvider), isA<NativeDeviceActions>());
      expect(createDeviceActions(), isA<NativeDeviceActions>());
      expect(c.read(displayNamesProvider), isA<BundledDisplayNames>());
    });

    test('the device languages come from the device actions, in their '
        'order', () {
      final c = ProviderContainer(
        overrides: [
          deviceActionsProvider.overrideWithValue(
            _Device(languageList: const ['ja-JP', 'th-TH', 'en']),
          ),
        ],
      );
      addTearDown(c.dispose);
      expect(c.read(deviceLanguagesProvider), ['ja-JP', 'th-TH', 'en']);
    });
  });
}
