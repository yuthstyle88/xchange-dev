import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/core/endpoint/kiosk_endpoint.dart';

void main() {
  group('KioskEndpoint whitespace in server address (#2897)', () {
    test(
      'Row 1: Server with tab inside throws EndpointFormatException(EndpointProblem.serverInvalid)',
      () {
        expect(
          () => KioskEndpoint.parse('shop.example.com\t8443', 'MAIN'),
          throwsA(
            isA<EndpointFormatException>().having(
              (e) => e.problem,
              'problem',
              EndpointProblem.serverInvalid,
            ),
          ),
        );
      },
    );

    test(
      'Row 2: Server with newline inside throws EndpointFormatException(EndpointProblem.serverInvalid)',
      () {
        expect(
          () => KioskEndpoint.parse('shop.example.com\n8443', 'MAIN'),
          throwsA(
            isA<EndpointFormatException>().having(
              (e) => e.problem,
              'problem',
              EndpointProblem.serverInvalid,
            ),
          ),
        );
      },
    );
  });
}
