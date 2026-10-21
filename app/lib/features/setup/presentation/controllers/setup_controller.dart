import 'dart:async';

import 'package:dio/dio.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../../core/endpoint/endpoint_store.dart';
import '../../../../core/endpoint/kiosk_endpoint.dart';
import '../../app/setup_providers.dart';

class SetupState {
  const SetupState({
    this.testing = false,
    this.tested,
    this.branchName,
    this.errorKey,
    this.serverMessage,
  });

  final bool testing;

  /// The endpoint whose test succeeded; the only one [SetupController.save]
  /// will store.
  final KioskEndpoint? tested;
  final String? branchName;

  /// A key of `setup.json` naming what is wrong, or null.
  final String? errorKey;

  /// The server's own sentence when it refused the test.
  final String? serverMessage;
}

/// Tests a server and branch typed on the setup screen and stores them only
/// once a test of exactly those values has succeeded (§8.1).
class SetupController extends Notifier<SetupState> {
  int _run = 0;

  @override
  SetupState build() => const SetupState();

  bool get _allowInsecure => ref.read(allowInsecureServerProvider);

  KioskEndpoint _parse(String server, String branch) =>
      KioskEndpoint.parse(server, branch, allowInsecure: _allowInsecure);

  /// Forgets the last result, as when a field is edited.
  void forget() {
    _run++;
    state = const SetupState();
  }

  Future<void> test(String server, String branch) async {
    final run = ++_run;
    final KioskEndpoint endpoint;
    try {
      endpoint = _parse(server, branch);
    } on EndpointFormatException catch (e) {
      state = SetupState(errorKey: e.problem.name);
      return;
    }
    state = const SetupState(testing: true);
    final probe = ref.read(setupProbeProvider);
    try {
      await probe(endpoint);
      if (run != _run) return;
      state = SetupState(tested: endpoint, branchName: endpoint.branchId);
    } on DioException catch (e) {
      if (run != _run) return;
      // Only the server's own answer counts as a refusal; dio's `message`
      // describes a missing answer and stays off the screen.
      final data = e.response?.data;
      final serverMessage = data is Map
          ? (data['message'] ?? data['error'])?.toString()
          : null;
      state = serverMessage != null && serverMessage.isNotEmpty
          ? SetupState(serverMessage: serverMessage)
          : const SetupState(errorKey: 'unreachable');
    } catch (_) {
      if (run != _run) return;
      state = const SetupState(errorKey: 'unreachable');
    }
  }

  /// Stores [server] and [branch] only if they are the values the last
  /// successful test used. Returns whether it did.
  Future<bool> save(String server, String branch) async {
    final tested = state.tested;
    KioskEndpoint? endpoint;
    try {
      endpoint = _parse(server, branch);
    } on EndpointFormatException {
      endpoint = null;
    }
    if (tested == null || state.testing || endpoint != tested) {
      state = SetupState(
        tested: tested,
        branchName: state.branchName,
        errorKey: 'notTested',
      );
      return false;
    }
    await ref.read(endpointStoreProvider).save(endpoint!);
    ref.read(kioskEndpointProvider.notifier).set(endpoint);
    return true;
  }
}

final setupControllerProvider = NotifierProvider<SetupController, SetupState>(
  SetupController.new,
);
