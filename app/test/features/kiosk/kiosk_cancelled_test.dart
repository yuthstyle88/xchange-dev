// KC-A (kiosk-cancel, docs/design/blueprint/kiosk-cancel.md §2, §3.6, §5 row KC-A): the kiosk app's cancelled
// status, the cancel token kept from confirm, and the customer's own cancel. Written before the implementation by
// the sub-controller claude/36d9dc6a. Do not edit, delete or weaken these tests; add tests in a file of your own.
//
// What these tests pin (wire shape: AG controller KC, #564 issuecomment-6307996084):
// - `KioskStatus.cancelled` with wire 'CANCELLED'.
// - `RequestIdentity.cancelToken` (String?): the `cancelToken` key of a confirm answer, or null when the key is
//   absent (a repeated confirm has no key, never a null value).
// - `KioskRepository.customerCancel(String id, String cancelToken)` → POST /api/kiosk
//   {"action":"customerCancel","payload":{"id","cancelToken","branchId"}} (KioskApi adds branchId), answering the
//   request identity with status CANCELLED.
// - Every customerCancel refusal is the same sentence, surfaced as a KioskFailure (as every refusal is).
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

  test('row 1: CANCELLED parses to KioskStatus.cancelled', () {
    expect(KioskStatus.parse('CANCELLED'), KioskStatus.cancelled);
    expect(KioskStatus.cancelled.wire, 'CANCELLED');
  });

  test(
    'row 2: the confirm answer keeps cancelToken; a missing key is null',
    () {
      final withToken = RequestIdentity.fromJson({
        'id': 'k-1',
        'queue': 'K001',
        'orderNo': 'ORD-1',
        'transactionNo': null,
        'status': 'WAITING',
        'cancelToken': 'b1f0c5e2-3a4d-4c8e-9f10-2a3b4c5d6e7f',
      });
      expect(withToken.cancelToken, 'b1f0c5e2-3a4d-4c8e-9f10-2a3b4c5d6e7f');

      final repeat = RequestIdentity.fromJson({
        'id': 'k-1',
        'queue': 'K001',
        'orderNo': 'ORD-1',
        'transactionNo': null,
        'status': 'WAITING',
      });
      expect(repeat.cancelToken, isNull);
    },
  );

  test(
    'row 3: confirm through the repository carries the cancelToken back',
    () async {
      adapter.body = {
        'id': 'k-1',
        'queue': 'K001',
        'orderNo': 'ORD-1',
        'transactionNo': null,
        'status': 'WAITING',
        'cancelToken': 'tok-1',
      };
      final id = await container.read(kioskRepositoryProvider).confirm('k-1');
      expect(id.status, KioskStatus.waiting);
      expect(id.cancelToken, 'tok-1');
    },
  );

  test(
    'row 4: customerCancel posts id, cancelToken and the branch, and answers CANCELLED',
    () async {
      adapter.body = {
        'id': 'k-1',
        'queue': 'K001',
        'orderNo': 'ORD-1',
        'transactionNo': null,
        'status': 'CANCELLED',
      };
      final id = await container
          .read(kioskRepositoryProvider)
          .customerCancel('k-1', 'tok-1');
      expect(adapter.last!.method, 'POST');
      expect('${adapter.last!.uri}', 'https://kiosk.test/api/kiosk');
      expect(jsonDecode(adapter.sent!), {
        'action': 'customerCancel',
        'payload': {'id': 'k-1', 'cancelToken': 'tok-1', 'branchId': 'B-7'},
      });
      expect(id.status, KioskStatus.cancelled);
    },
  );

  test(
    'row 5: a customerCancel refusal surfaces the server sentence as a KioskFailure',
    () {
      adapter
        ..status = 400
        ..body = {'error': 'ยกเลิกคิวที่ตู้ไม่ได้ กรุณาติดต่อพนักงาน'};
      expect(
        container.read(kioskRepositoryProvider).customerCancel('k-1', 'tok-1'),
        throwsA(
          isA<KioskFailure>().having(
            (f) => f.serverMessage,
            'serverMessage',
            'ยกเลิกคิวที่ตู้ไม่ได้ กรุณาติดต่อพนักงาน',
          ),
        ),
      );
    },
  );
}
