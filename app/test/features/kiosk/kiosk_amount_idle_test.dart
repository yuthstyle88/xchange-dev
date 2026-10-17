import 'package:fake_async/fake_async.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/features/kiosk/app/kiosk_providers.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/controllers/kiosk_controller.dart';

import 'kiosk_controller_test.dart' show FakeRepository, l10n;

void main() {
  late FakeRepository server;
  late ProviderContainer container;
  late KioskController c;

  KioskState st() => container.read(kioskControllerProvider);

  KioskController make() {
    container = ProviderContainer(
      overrides: [
        kioskRepositoryProvider.overrideWithValue(server),
        kioskL10nProvider.overrideWithValue(l10n()),
        deviceLanguagesProvider.overrideWithValue(const ['en']),
      ],
    );
    container.listen(kioskControllerProvider, (_, _) {});
    return container.read(kioskControllerProvider.notifier);
  }

  setUp(() => server = FakeRepository());

  test(
    'Idle timer at 1:50, customer edits the amount (setAmount), 30 s more pass -> No reset; amount kept',
    () {
      fakeAsync((async) {
        c = make();
        c.start();
        async.flushMicrotasks();

        final gen = st().resetGeneration;

        // 1 minute 50 seconds elapse
        async.elapse(const Duration(minutes: 1, seconds: 50));
        expect(st().resetGeneration, gen);

        // Customer edits the amount
        c.setAmount('500');
        expect(st().amountText, '500');

        // 30 seconds more pass (total 2m 20s from start, but only 30s since edit)
        async.elapse(const Duration(seconds: 30));
        async.flushMicrotasks();

        // No reset; amount kept
        expect(st().screen, KioskScreen.select);
        expect(st().amountText, '500');
        expect(st().resetGeneration, gen);

        container.dispose();
      });
    },
  );

  test(
    'No edits for 2 minutes after the last amount change -> Idle reset as today',
    () {
      fakeAsync((async) {
        c = make();
        c.start();
        async.flushMicrotasks();

        final gen = st().resetGeneration;

        // 1 minute 50 seconds elapse
        async.elapse(const Duration(minutes: 1, seconds: 50));

        // Customer edits the amount
        c.setAmount('500');
        expect(st().amountText, '500');

        // Wait 1 minute 50 seconds after the amount change (total 3m 40s from start):
        // Still within the 2-minute window since the last amount change, so no reset yet.
        async.elapse(const Duration(minutes: 1, seconds: 50));
        expect(st().amountText, '500');
        expect(st().resetGeneration, gen);

        // 10 more seconds pass (reaching full 2 minutes with no edits after setAmount)
        async.elapse(const Duration(seconds: 10));
        async.flushMicrotasks();

        // Idle reset as today: amount cleared and reset generation incremented
        expect(st().amountText, '');
        expect(st().resetGeneration, gen + 1);

        container.dispose();
      });
    },
  );
}
