import 'dart:convert';
import 'dart:typed_data';

import 'package:dio/dio.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/core/network/dio_provider.dart';
import 'package:xchange_kiosk/features/kiosk/app/kiosk_providers.dart';
import 'package:xchange_kiosk/features/kiosk/domain/entities/kiosk_models.dart';
import 'package:xchange_kiosk/features/kiosk/domain/kiosk_failure.dart';

/// Answers every request with [status] and [body], and keeps what was sent.
class _Adapter implements HttpClientAdapter {
  int status = 200;
  Object body = const {};
  RequestOptions? last;
  String? sent;

  @override
  Future<ResponseBody> fetch(
    RequestOptions options,
    Stream<Uint8List>? requestStream,
    Future<void>? cancelFuture,
  ) async {
    last = options;
    sent = requestStream == null
        ? null
        : utf8.decode(await requestStream.expand((c) => c).toList());
    return ResponseBody.fromString(
      body is String ? body as String : jsonEncode(body),
      status,
      headers: {
        Headers.contentTypeHeader: [Headers.jsonContentType],
      },
    );
  }

  @override
  void close({bool force = false}) {}
}

void main() {
  late _Adapter adapter;
  late ProviderContainer container;

  setUp(() {
    adapter = _Adapter();
    container = ProviderContainer(
      overrides: [
        apiBaseUrlProvider.overrideWithValue('https://kiosk.test'),
        kioskBranchProvider.overrideWithValue('B-7'),
      ],
    );
    container.read(dioProvider).httpClientAdapter = adapter;
  });
  tearDown(() => container.dispose());

  test('a request names the action, carries the branch and passes the '
      'server request check', () async {
    adapter.body = {
      'id': 'x',
      'queue': 'K-X',
      'orderNo': 'ORD-X',
      'transactionNo': null,
      'status': 'QUOTED',
      'paymentMethod': 'Cash',
      'type': 'BUY',
      'currency': 'USD',
      'rate': 32.1,
      'foreign': 100,
      'thb': 3210,
    };
    final quote = await container
        .read(kioskRepositoryProvider)
        .quote(
          type: ExchangeType.buy,
          currency: 'USD',
          amount: '100',
          paymentMethod: 'Cash',
        );
    expect(adapter.last!.method, 'POST');
    expect('${adapter.last!.uri}', 'https://kiosk.test/api/kiosk');
    expect(adapter.last!.headers['X-Xchange-Request'], '1');
    expect(jsonDecode(adapter.sent!), {
      'action': 'quote',
      'payload': {
        'type': 'BUY',
        'currency': 'USD',
        'amount': '100',
        'paymentMethod': 'Cash',
        'branchId': 'B-7',
      },
    });
    expect(quote.thb, 3210);
    expect(quote.identity.status, KioskStatus.quoted);
  });

  test('a refusal surfaces the server sentence as a KioskFailure', () {
    adapter
      ..status = 400
      ..body = {'error': 'Please review your quote for 3 seconds.'};
    expect(
      container.read(kioskRepositoryProvider).confirm('x'),
      throwsA(
        isA<KioskFailure>().having(
          (f) => f.serverMessage,
          'serverMessage',
          'Please review your quote for 3 seconds.',
        ),
      ),
    );
  });

  test('an answer that is not the agreed shape is a failure with no message, '
      'never a crash', () {
    adapter.body = '[1,2,3]';
    expect(
      container.read(kioskRepositoryProvider).rates(),
      throwsA(
        isA<KioskFailure>().having((f) => f.serverMessage, 'serverMessage', ''),
      ),
    );
  });

  test('a status this client has never heard of does not break parsing', () {
    final id = RequestIdentity.fromJson({
      'id': 'x',
      'queue': 'K-X',
      'orderNo': 'ORD-X',
      'status': 'ON_HOLD',
    });
    expect(id.status, KioskStatus.unknown);
    expect(id.rawStatus, 'ON_HOLD');
  });
}
