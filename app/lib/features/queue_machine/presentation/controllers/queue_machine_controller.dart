import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../infrastructure/queue_machine_api.dart';
import '../../infrastructure/staff_session.dart';
import '../../printing/printer_providers.dart';
import '../../printing/ticket_printer.dart';

enum QmScreen { signIn, scanner, issued }

enum PrintStatus { none, printed, failed, noPrinter }

class QueueMachineState {
  const QueueMachineState({
    this.screen = QmScreen.signIn,
    this.displayNumber,
    this.print = PrintStatus.none,
    this.error,
  });

  final QmScreen screen;
  final String? displayNumber;
  final PrintStatus print;
  final String? error;
}

/// Parses a customer request QR URL `<origin>/q#<token>` (§4 D3, §5r R1).
/// Returns the fragment token when the URL has a scheme/authority, path `/q`,
/// and a non-empty fragment; returns `null` for anything else.
String? parseRequestQr(String raw) {
  final trimmed = raw.trim();
  if (trimmed.isEmpty) return null;
  final uri = Uri.tryParse(trimmed);
  if (uri == null || !uri.hasScheme || !uri.hasAuthority) return null;
  if (uri.path != '/q') return null;
  if (!uri.hasFragment || uri.fragment.isEmpty) return null;
  return uri.fragment;
}

class QueueMachineController extends Notifier<QueueMachineState> {
  Timer? _timer;
  int _scanSeq = 0;

  @override
  QueueMachineState build() {
    ref.onDispose(() {
      _timer?.cancel();
      _timer = null;
    });
    return const QueueMachineState();
  }

  Future<void> signIn(String id, String pw) async {
    final trimmedId = id.trim();
    if (trimmedId.isEmpty || pw.isEmpty) {
      state = const QueueMachineState(
        screen: QmScreen.signIn,
        error: 'invalidCredentials',
      );
      return;
    }
    try {
      await ref.read(queueMachineApiProvider).signIn(trimmedId, pw);
      _timer?.cancel();
      state = const QueueMachineState(screen: QmScreen.scanner);
    } on QueueMachineRefused catch (e) {
      state = QueueMachineState(screen: QmScreen.signIn, error: e.code);
    } catch (_) {
      state = const QueueMachineState(
        screen: QmScreen.signIn,
        error: 'signInFailed',
      );
    }
  }

  Future<void> signOut() async {
    _timer?.cancel();
    _scanSeq += 1;
    state = const QueueMachineState(screen: QmScreen.signIn);
    try {
      await ref.read(staffSessionProvider).signOut();
    } catch (_) {
      // Ignore network errors after returning the machine to sign-in.
    }
  }

  Future<void> onScan(String raw) async {
    if (state.screen == QmScreen.signIn) return;

    final token = parseRequestQr(raw);
    if (token == null) {
      _timer?.cancel();
      state = const QueueMachineState(
        screen: QmScreen.scanner,
        error: 'invalidQr',
      );
      return;
    }

    _timer?.cancel();
    final seq = ++_scanSeq;
    final api = ref.read(queueMachineApiProvider);

    final IssuedTicket issued;
    try {
      issued = await api.issueByToken(token);
    } on QueueMachineRefused catch (e) {
      if (_scanSeq != seq) return;
      state = QueueMachineState(screen: QmScreen.scanner, error: e.code);
      return;
    } catch (_) {
      if (_scanSeq != seq) return;
      state = const QueueMachineState(
        screen: QmScreen.scanner,
        error: 'issueFailed',
      );
      return;
    }

    if (_scanSeq != seq) return;
    final bytes = Uint8List.fromList(base64Decode(issued.ticketEscpos));
    var printStatus = PrintStatus.printed;
    try {
      await ref.read(ticketPrinterProvider).write(bytes);
    } on PrinterUnavailable catch (e) {
      if (e.reason == 'noDevice' || e.reason == 'permissionRefused') {
        printStatus = PrintStatus.noPrinter;
      } else {
        printStatus = PrintStatus.failed;
      }
    } catch (_) {
      printStatus = PrintStatus.failed;
    }

    if (_scanSeq != seq) return;
    var seconds = 10;
    try {
      final configured = await api.resultSeconds();
      if (configured > 0) seconds = configured;
    } catch (_) {
      seconds = 10;
    }

    if (_scanSeq != seq) return;
    state = QueueMachineState(
      screen: QmScreen.issued,
      displayNumber: issued.displayNumber,
      print: printStatus,
      error: null,
    );

    _timer?.cancel();
    _timer = Timer(Duration(seconds: seconds), () {
      if (_scanSeq == seq && state.screen == QmScreen.issued) {
        state = const QueueMachineState(screen: QmScreen.scanner);
      }
    });
  }
}

final queueMachineControllerProvider =
    NotifierProvider<QueueMachineController, QueueMachineState>(
      QueueMachineController.new,
    );
