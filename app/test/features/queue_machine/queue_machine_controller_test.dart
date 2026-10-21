// QM-2 (kiosk-shared-phone §5r F2–F4, F6; rows A1, A2, A3, A5–A8): the staff queue machine's controller.
// Written before the implementation by the sub-controller claude/36d9dc6a. Do not edit, delete or weaken these
// tests; add tests in a file of your own.
//
// What these tests pin (QM-2 implements them; TicketPrinter / PrinterUnavailable come from QM-3):
// - `lib/features/queue_machine/infrastructure/queue_machine_api.dart`:
//     class IssuedTicket { const IssuedTicket({required this.displayNumber, required this.ticketId,
//       required this.ticketEscpos}); final String displayNumber, ticketId, ticketEscpos; }  // base64 (§4.3a W8)
//     class QueueMachineRefused implements Exception { const QueueMachineRefused(this.code); final String code; }
//     abstract class QueueMachineApi {
//       Future<void> signIn(String identifier, String password);   // POST /api/auth/login (W6)
//       Future<IssuedTicket> issueByToken(String token);           // POST /api/kiosk issueByToken (W8)
//       Future<int> resultSeconds();                                // kiosk_settings result_seconds (§4 D14)
//     }
//     final queueMachineApiProvider = Provider<QueueMachineApi>(...);
// - `lib/features/queue_machine/presentation/controllers/queue_machine_controller.dart`:
//     enum QmScreen { signIn, scanner, issued }
//     enum PrintStatus { none, printed, failed, noPrinter }
//     class QueueMachineState { QmScreen screen; String? displayNumber; PrintStatus print; String? error; }
//     final queueMachineControllerProvider = NotifierProvider<QueueMachineController, QueueMachineState>(...);
//     class QueueMachineController { Future<void> signIn(String id, String pw); Future<void> onScan(String raw); }
//   It reads the printer from QM-3's `ticketPrinterProvider`.
import 'dart:convert';
import 'dart:typed_data';

import 'package:fake_async/fake_async.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/features/queue_machine/infrastructure/queue_machine_api.dart';
import 'package:xchange_kiosk/features/queue_machine/presentation/controllers/queue_machine_controller.dart';
import 'package:xchange_kiosk/features/queue_machine/printing/printer_providers.dart';
import 'package:xchange_kiosk/features/queue_machine/printing/ticket_printer.dart';

class _Api implements QueueMachineApi {
  final signIns = <String>[];
  final issued = <String>[];
  Object? issueError;
  int seconds = 10;
  final bytes = Uint8List.fromList([
    0x1B,
    0x40,
    0x4B,
    0x30,
    0x34,
    0x32,
    0x0A,
    0x1D,
    0x56,
    0x00,
  ]);

  @override
  Future<void> signIn(String identifier, String password) async =>
      signIns.add(identifier);

  @override
  Future<IssuedTicket> issueByToken(String token) async {
    issued.add(token);
    final e = issueError;
    if (e != null) throw e;
    return IssuedTicket(
      displayNumber: 'K042',
      ticketId: 't-1',
      ticketEscpos: base64Encode(bytes),
    );
  }

  @override
  Future<int> resultSeconds() async => seconds;
}

class _Printer implements TicketPrinter {
  _Printer({this.error});
  final Object? error;
  final writes = <Uint8List>[];

  @override
  Future<void> write(Uint8List bytes) async {
    final e = error;
    if (e != null) throw e;
    writes.add(Uint8List.fromList(bytes));
  }
}

ProviderContainer _container(_Api api, _Printer printer) {
  final c = ProviderContainer(
    overrides: [
      queueMachineApiProvider.overrideWithValue(api),
      ticketPrinterProvider.overrideWithValue(printer),
    ],
  );
  c.listen(queueMachineControllerProvider, (_, _) {});
  return c;
}

QueueMachineState _s(ProviderContainer c) =>
    c.read(queueMachineControllerProvider);
QueueMachineController _ctl(ProviderContainer c) =>
    c.read(queueMachineControllerProvider.notifier);

void _signedIn(FakeAsync async, ProviderContainer c) {
  _ctl(c).signIn('teller1', 'secret');
  async.flushMicrotasks();
}

void main() {
  test(
    'A1: no staff session -> the sign-in screen; a scan before sign-in sends nothing',
    () {
      fakeAsync((async) {
        final api = _Api();
        final c = _container(api, _Printer());
        expect(_s(c).screen, QmScreen.signIn);
        _ctl(c).onScan('https://h/q#tok');
        async.flushMicrotasks();
        expect(api.issued, isEmpty);
        c.dispose();
      });
    },
  );

  test('sign-in moves to the scanner', () {
    fakeAsync((async) {
      final api = _Api();
      final c = _container(api, _Printer());
      _signedIn(async, c);
      expect(api.signIns, ['teller1']);
      expect(_s(c).screen, QmScreen.scanner);
      c.dispose();
    });
  });

  test(
    'A2 + A3: a /q#tok scan issues once, shows the number, and prints the decoded bytes unchanged',
    () {
      fakeAsync((async) {
        final api = _Api();
        final printer = _Printer();
        final c = _container(api, printer);
        _signedIn(async, c);
        _ctl(c).onScan('https://xc-v3.spacing.cloud/q#tok');
        async.flushMicrotasks();
        expect(api.issued, ['tok']);
        expect(_s(c).screen, QmScreen.issued);
        expect(_s(c).displayNumber, 'K042');
        expect(_s(c).print, PrintStatus.printed);
        expect(printer.writes, hasLength(1));
        expect(printer.writes.single, orderedEquals(api.bytes));
        c.dispose();
      });
    },
  );

  test(
    'A5: 404 notFound -> one refusal, back on the scanner, nothing printed',
    () {
      fakeAsync((async) {
        final api = _Api()..issueError = const QueueMachineRefused('notFound');
        final printer = _Printer();
        final c = _container(api, printer);
        _signedIn(async, c);
        _ctl(c).onScan('https://h/q#unknown');
        async.flushMicrotasks();
        expect(_s(c).screen, QmScreen.scanner);
        expect(_s(c).error, isNotNull);
        expect(printer.writes, isEmpty);
        c.dispose();
      });
    },
  );

  test('A6: the issued screen returns to the scanner after result_seconds', () {
    fakeAsync((async) {
      final api = _Api()..seconds = 10;
      final c = _container(api, _Printer());
      _signedIn(async, c);
      _ctl(c).onScan('https://h/q#tok');
      async.flushMicrotasks();
      async.elapse(const Duration(seconds: 9));
      async.flushMicrotasks();
      expect(_s(c).screen, QmScreen.issued);
      async.elapse(const Duration(seconds: 1));
      async.flushMicrotasks();
      expect(_s(c).screen, QmScreen.scanner);
      expect(_s(c).displayNumber, isNull);
      c.dispose();
    });
  });

  test(
    'A7: anything that is not /q#<token> is refused on the device, with nothing sent',
    () {
      fakeAsync((async) {
        final api = _Api();
        final c = _container(api, _Printer());
        _signedIn(async, c);
        for (final raw in [
          'test-qr-token-xyz',
          'https://h/q#',
          'https://h/c#v1.x',
          '',
        ]) {
          _ctl(c).onScan(raw);
          async.flushMicrotasks();
        }
        expect(api.issued, isEmpty);
        expect(_s(c).screen, QmScreen.scanner);
        c.dispose();
      });
    },
  );

  test(
    'A8: no printer (PrinterUnavailable) -> the number still shows, print = noPrinter',
    () {
      fakeAsync((async) {
        final api = _Api();
        final c = _container(
          api,
          _Printer(error: const PrinterUnavailable('noDevice')),
        );
        _signedIn(async, c);
        _ctl(c).onScan('https://h/q#tok');
        async.flushMicrotasks();
        expect(_s(c).screen, QmScreen.issued);
        expect(_s(c).displayNumber, 'K042');
        expect(_s(c).print, PrintStatus.noPrinter);
        c.dispose();
      });
    },
  );

  test(
    'A4: a failed write -> print = failed; a re-scan issues again (same bytes from the server)',
    () {
      fakeAsync((async) {
        final api = _Api();
        final c = _container(
          api,
          _Printer(error: const PrinterUnavailable('writeFailed')),
        );
        _signedIn(async, c);
        _ctl(c).onScan('https://h/q#tok');
        async.flushMicrotasks();
        expect(_s(c).print, PrintStatus.failed);
        _ctl(c).onScan('https://h/q#tok');
        async.flushMicrotasks();
        expect(api.issued, ['tok', 'tok']);
        c.dispose();
      });
    },
  );
}
