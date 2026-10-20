import 'dart:convert';
import 'dart:typed_data';

import 'package:dio/dio.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/core/network/dio_provider.dart';
import 'package:xchange_kiosk/features/kiosk/app/kiosk_providers.dart';
import 'package:xchange_kiosk/features/kiosk/domain/kiosk_failure.dart';

class _ExtraAdapter implements HttpClientAdapter {
  int status = 200;
  Object? body = const {};
  DioExceptionType? throwType;
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
    if (throwType != null) {
      throw DioException(requestOptions: options, type: throwType!);
    }
    return ResponseBody.fromString(
      body == null ? '' : (body is String ? body as String : jsonEncode(body)),
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
  late _ExtraAdapter adapter;
  late ProviderContainer container;

  setUp(() {
    adapter = _ExtraAdapter();
    container = ProviderContainer(
      overrides: [
        apiBaseUrlProvider.overrideWithValue('https://kiosk.test'),
        kioskBranchProvider.overrideWithValue('B-7'),
      ],
    );
    container.read(dioProvider).httpClientAdapter = adapter;
  });
  tearDown(() => container.dispose());

  test('a 500 with no body throws KioskFailure with no message', () async {
    adapter
      ..status = 500
      ..body = null;
    await expectLater(
      container.read(kioskBookingRepositoryProvider).checkInBooking('tok-123'),
      throwsA(
        isA<KioskFailure>().having((f) => f.serverMessage, 'serverMessage', ''),
      ),
    );
  });

  test(
    'no connection (DioExceptionType.connectionError) throws KioskFailure with no message',
    () async {
      adapter.throwType = DioExceptionType.connectionError;
      await expectLater(
        container
            .read(kioskBookingRepositoryProvider)
            .checkInBooking('tok-123'),
        throwsA(
          isA<KioskFailure>().having(
            (f) => f.serverMessage,
            'serverMessage',
            '',
          ),
        ),
      );
    },
  );

  test(
    '200 with ticketNumber missing throws KioskFailure with no message',
    () async {
      adapter.body = {
        'id': 'bk-1',
        'status': 'CHECKED_IN',
        'bookingNo': 'BK-2026-0042',
      };
      await expectLater(
        container
            .read(kioskBookingRepositoryProvider)
            .checkInBooking('tok-123'),
        throwsA(
          isA<KioskFailure>().having(
            (f) => f.serverMessage,
            'serverMessage',
            '',
          ),
        ),
      );
    },
  );

  test('an empty qrToken is sent as given without a client rule', () async {
    adapter.body = {
      'id': 'bk-empty',
      'status': 'CHECKED_IN',
      'ticketNumber': 'B-099',
      'bookingNo': null,
    };
    final res = await container
        .read(kioskBookingRepositoryProvider)
        .checkInBooking('');
    expect(jsonDecode(adapter.sent!), {'branch': 'B-7', 'qrToken': ''});
    expect(res.id, 'bk-empty');
    expect(res.ticketNumber, 'B-099');
  });
}
