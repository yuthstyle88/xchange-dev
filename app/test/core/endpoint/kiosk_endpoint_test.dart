// Provided with ticket A1. These tests are the specification: make them pass,
// do not edit, delete or weaken them.
import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/core/endpoint/kiosk_endpoint.dart';

EndpointProblem problemOf(void Function() parse) {
  try {
    parse();
  } on EndpointFormatException catch (e) {
    return e.problem;
  }
  fail('expected an EndpointFormatException');
}

void main() {
  group('the server address', () {
    test('a bare host is taken as https', () {
      final e = KioskEndpoint.parse('xc-customer.example.com', 'MAIN');
      expect(e.origin, 'https://xc-customer.example.com');
      expect(e.serverOrigin, Uri.parse('https://xc-customer.example.com'));
      expect(e.branchId, 'MAIN');
    });

    test('an https address keeps its host and a non-default port', () {
      expect(
        KioskEndpoint.parse('https://shop.example.com:8443', 'MAIN').origin,
        'https://shop.example.com:8443',
      );
      expect(
        KioskEndpoint.parse('https://shop.example.com:443', 'MAIN').origin,
        'https://shop.example.com',
      );
    });

    test('a pasted page address is cut down to its origin', () {
      expect(
        KioskEndpoint.parse(
          'https://shop.example.com/kiosk/?branch=B-7#top',
          'MAIN',
        ).origin,
        'https://shop.example.com',
      );
    });

    test('spaces around it and the case of the host do not matter', () {
      expect(
        KioskEndpoint.parse('  HTTPS://Shop.Example.COM/  ', 'MAIN').origin,
        'https://shop.example.com',
      );
    });

    test('nothing typed is told apart from something wrong', () {
      expect(
        problemOf(() => KioskEndpoint.parse('', 'MAIN')),
        EndpointProblem.serverMissing,
      );
      expect(
        problemOf(() => KioskEndpoint.parse('   ', 'MAIN')),
        EndpointProblem.serverMissing,
      );
    });

    test('what is not a host is refused', () {
      for (final text in [
        'https://',
        'http://',
        'shop example.com',
        'ftp://shop.example.com',
        'https://user:secret@shop.example.com',
        '://shop.example.com',
      ]) {
        expect(
          problemOf(() => KioskEndpoint.parse(text, 'MAIN')),
          EndpointProblem.serverInvalid,
          reason: text,
        );
      }
    });

    test('plain http is refused unless the build allows it', () {
      expect(
        problemOf(
          () => KioskEndpoint.parse('http://192.168.1.75:8095', 'MAIN'),
        ),
        EndpointProblem.serverNotSecure,
      );
      expect(
        KioskEndpoint.parse(
          'http://192.168.1.75:8095',
          'MAIN',
          allowInsecure: true,
        ).origin,
        'http://192.168.1.75:8095',
      );
      expect(
        KioskEndpoint.parse(
          'http://192.168.1.75:80',
          'MAIN',
          allowInsecure: true,
        ).origin,
        'http://192.168.1.75',
      );
    });

    test('allowing http never turns a bare host into http', () {
      expect(
        KioskEndpoint.parse(
          'shop.example.com',
          'MAIN',
          allowInsecure: true,
        ).origin,
        'https://shop.example.com',
      );
    });
  });

  group('the branch code', () {
    test('is kept as typed, without the spaces around it', () {
      expect(KioskEndpoint.parse('a.example', ' B-7 ').branchId, 'B-7');
      // v2 branch ids are case-sensitive (`MAIN`, `B<request id>`).
      expect(KioskEndpoint.parse('a.example', 'main').branchId, 'main');
    });

    test('nothing typed is told apart from something wrong', () {
      expect(
        problemOf(() => KioskEndpoint.parse('a.example', '')),
        EndpointProblem.branchMissing,
      );
      expect(
        problemOf(() => KioskEndpoint.parse('a.example', '  ')),
        EndpointProblem.branchMissing,
      );
    });

    test('a code with a space inside, or longer than 80, is refused', () {
      expect(
        problemOf(() => KioskEndpoint.parse('a.example', 'B 7')),
        EndpointProblem.branchInvalid,
      );
      expect(
        problemOf(() => KioskEndpoint.parse('a.example', 'B' * 81)),
        EndpointProblem.branchInvalid,
      );
      expect(KioskEndpoint.parse('a.example', 'B' * 80).branchId, 'B' * 80);
    });

    test('the server is judged before the branch', () {
      expect(
        problemOf(() => KioskEndpoint.parse('', '')),
        EndpointProblem.serverMissing,
      );
    });
  });

  test('two endpoints with the same origin and branch are equal', () {
    final a = KioskEndpoint.parse('https://shop.example.com/', 'B-7');
    final b = KioskEndpoint.parse('shop.example.com', 'B-7');
    expect(a, b);
    expect(a.hashCode, b.hashCode);
    expect(a, isNot(KioskEndpoint.parse('shop.example.com', 'B-8')));
    expect(a, isNot(KioskEndpoint.parse('other.example.com', 'B-7')));
  });
}
