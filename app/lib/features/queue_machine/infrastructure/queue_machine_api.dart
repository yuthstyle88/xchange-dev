import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'queue_machine_api_dio.dart';

class IssuedTicket {
  const IssuedTicket({
    required this.displayNumber,
    required this.ticketId,
    required this.ticketEscpos,
  });

  final String displayNumber;
  final String ticketId;
  final String ticketEscpos;
}

class QueueMachineRefused implements Exception {
  const QueueMachineRefused(this.code);

  final String code;

  @override
  String toString() => 'QueueMachineRefused($code)';
}

abstract class QueueMachineApi {
  Future<void> signIn(String identifier, String password);
  Future<IssuedTicket> issueByToken(String token);
  Future<int> resultSeconds();
}

final queueMachineApiProvider = Provider<QueueMachineApi>(
  buildDioQueueMachineApi,
);
