// Kiosk booking QR check-in, data layer (survey xchange#564 issuecomment-6262359159 row 24, part a).
// Written before the implementation by the sub-controller (claude/dell-claude-3). Do not edit, delete or
// weaken; add tests in a file of your own. UNVERIFIED: written without a Flutter run (no build host).
//
// Design: `docs/design/blueprint/booking-slots.md:90` (check in by QR at the kiosk, QR only), `:223`
// (`kioskBookingCheckIn` body `qrToken`, `POST /api/kiosk/bookings/check-in`, kiosk path).
// The wire is `api/src/presentation/http/v3/commands/bookings/kiosk_check_in.rs`: input `{ branch, qrToken }`
// (`:136-143`), answer `{ id, status, ticketNumber, bookingNo }` (`:125-130`), refusals `qrMismatch` /
// `checkInWindow` / `queueClosed` (400) and `notFound` (404) as `{ "error": { "code", "message" } }`.
//
// The API this test pins ([required]):
//   KioskRepository.checkInBooking(String qrToken) -> Future<BookingCheckInModel>
//   BookingCheckInModel { String id; String status; String ticketNumber; String? bookingNo; }
//       (in `lib/features/kiosk/domain/entities/kiosk_models.dart`)
//   a refusal throws `KioskFailure` whose `serverMessage` is the server's `error.code`
//   (the v3 answer's error is an object, not the text `/api/kiosk` sends);
//   no answer (offline / malformed) throws `KioskFailure('')`, as every other kiosk call does.
import 'dart:convert';
import 'dart:typed_data';

import 'package:dio/dio.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/core/network/dio_provider.dart';
import 'package:xchange_kiosk/features/kiosk/app/kiosk_providers.dart';
import 'package:xchange_kiosk/features/kiosk/domain/kiosk_failure.dart';

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

  test('posts the QR token with the branch to /api/kiosk/bookings/check-in and reads the ticket', () async {
    adapter.body = {
      'id': 'bk-1',
      'status': 'CHECKED_IN',
      'ticketNumber': 'B-012',
      'bookingNo': 'BK-2026-0042',
    };
    final res = await container.read(kioskRepositoryProvider).checkInBooking('tok-123');

    expect(adapter.last?.method, 'POST');
    expect(adapter.last?.path, '/api/kiosk/bookings/check-in');
    expect(jsonDecode(adapter.sent!), {'branch': 'B-7', 'qrToken': 'tok-123'});
    expect(res.id, 'bk-1');
    expect(res.status, 'CHECKED_IN');
    expect(res.ticketNumber, 'B-012');
    expect(res.bookingNo, 'BK-2026-0042');
  });

  test('a missing bookingNo reads as null', () async {
    adapter.body = {'id': 'bk-2', 'status': 'CHECKED_IN', 'ticketNumber': 'B-013', 'bookingNo': null};
    final res = await container.read(kioskRepositoryProvider).checkInBooking('tok-9');
    expect(res.bookingNo, isNull);
  });

  for (final c in [
    (400, 'qrMismatch'),
    (400, 'checkInWindow'),
    (400, 'queueClosed'),
    (404, 'notFound'),
  ]) {
    test('a ${c.$1} ${c.$2} refusal throws KioskFailure carrying the code', () async {
      adapter
        ..status = c.$1
        ..body = {
          'error': {'code': c.$2, 'message': c.$2.toUpperCase()},
        };
      await expectLater(
        container.read(kioskRepositoryProvider).checkInBooking('tok-123'),
        throwsA(isA<KioskFailure>().having((f) => f.serverMessage, 'serverMessage', c.$2)),
      );
    });
  }

  test('an answer that is not an object throws KioskFailure with no message', () async {
    adapter.body = 'not json';
    await expectLater(
      container.read(kioskRepositoryProvider).checkInBooking('tok-123'),
      throwsA(isA<KioskFailure>().having((f) => f.serverMessage, 'serverMessage', '')),
    );
  });
}
