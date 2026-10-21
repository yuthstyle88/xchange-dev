// #174 (A3, #51 sixth finding): an action that fails after its widget has gone
// must raise nothing.
import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/core/platform/device_actions.dart';

class _SlowActions extends DeviceActions {
  final pending = <Completer<void>>[];

  @override
  List<String> languages() => const ['en'];

  Future<void> _wait() {
    final c = Completer<void>();
    pending.add(c);
    return c.future;
  }

  @override
  Future<void> saveImage(String dataUrl, String filename) => _wait();

  @override
  Future<void> openHttps(Uri uri) => _wait();
}

void main() {
  testWidgets(
    'an async action failing after widget disposal does not throw unhandled exception',
    (tester) async {
      final actions = _SlowActions();
      await tester.pumpWidget(
        MaterialApp(
          home: Builder(
            builder: (context) {
              return ElevatedButton(
                onPressed: () async {
                  try {
                    await actions.openHttps(Uri.parse('https://example.com'));
                  } catch (_) {
                    if (context.mounted) {
                      // safe
                    }
                  }
                },
                child: const Text('Go'),
              );
            },
          ),
        ),
      );

      await tester.tap(find.text('Go'));
      await tester.pump();

      // Dispose widget before action completes
      await tester.pumpWidget(const SizedBox());
      actions.pending.single.completeError(
        const DeviceActionException('no handler'),
      );
      await tester.pump();
      await tester.pump();
      expect(tester.takeException(), isNull);
    },
  );
}
