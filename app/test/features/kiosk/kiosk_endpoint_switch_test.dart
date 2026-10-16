import 'dart:async';

import 'package:fake_async/fake_async.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/core/endpoint/endpoint_store.dart';
import 'package:xchange_kiosk/core/endpoint/kiosk_endpoint.dart';
import 'package:xchange_kiosk/features/kiosk/app/kiosk_providers.dart';
import 'package:xchange_kiosk/features/kiosk/domain/entities/kiosk_models.dart';
import 'package:xchange_kiosk/features/kiosk/domain/kiosk_failure.dart';
import 'package:xchange_kiosk/features/kiosk/domain/repository/kiosk_repository.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/controllers/kiosk_controller.dart';
import 'package:xchange_kiosk/features/setup/app/setup_providers.dart';
import 'package:xchange_kiosk/features/setup/presentation/controllers/setup_controller.dart';
import 'package:xchange_kiosk/l10n/kiosk_l10n.dart';

class _MemoryEndpointStore implements EndpointStore {
  _MemoryEndpointStore([this.stored]);
  KioskEndpoint? stored;

  @override
  Future<KioskEndpoint?> load() async => stored;

  @override
  Future<void> save(KioskEndpoint endpoint) async {
    stored = endpoint;
  }

  @override
  Future<void> clear() async => stored = null;

  @override
  String? originHeader(KioskEndpoint endpoint) => endpoint.origin;
}

const _usdA = {'code': 'USD', 'nameEn': 'US Dollar', 'buy': 32.1, 'sell': 32.6};
const _eurB = {'code': 'EUR', 'nameEn': 'Euro', 'buy': 35.0, 'sell': 35.5};

RatesModel _branchARates() => RatesModel.fromJson({
  'branchId': 'BRANCH-A',
  'branchName': 'Siam Branch A',
  'open': true,
  'rates': [_usdA],
  'paymentMethods': [
    {
      'id': 'Cash',
      'enabled': true,
      'buy': true,
      'sell': true,
      'instruction': 'Cash at A',
    },
    {
      'id': 'Bank Transfer',
      'enabled': true,
      'buy': true,
      'sell': true,
      'instruction': 'Transfer at A',
    },
  ],
});

RatesModel _branchBRates() => RatesModel.fromJson({
  'branchId': 'BRANCH-B',
  'branchName': 'Silom Branch B',
  'open': true,
  'rates': [_eurB],
  'paymentMethods': [
    {
      'id': 'Cash',
      'enabled': true,
      'buy': true,
      'sell': true,
      'instruction': 'Cash only at B',
    },
  ],
});

class _SwitchableBranchRepo implements KioskRepository {
  _SwitchableBranchRepo(this.branchId, this.onRates);

  final String branchId;
  final Future<RatesModel> Function(String branchId) onRates;

  @override
  Future<RatesModel> rates() => onRates(branchId);

  @override
  dynamic noSuchMethod(Invocation invocation) => throw UnimplementedError();
}

KioskL10n _l10n() => KioskL10n.fromJson({
  'langs': [
    {'code': 'en', 'lang': 'en', 'name': 'English'},
    {'code': 'th', 'lang': 'th', 'name': 'ไทย'},
  ],
  'text': {
    'en': {
      'closed': 'closed-en',
      'errGeneric': 'generic-en',
      'quoteExpired': 'expired-en',
      'statusError': 'status-error-en',
    },
    'th': {'closed': 'closed-th', 'errGeneric': 'generic-th'},
  },
  'errors': <String, String>{},
});

void main() {
  final endpointA = KioskEndpoint.parse('https://a.example.com', 'BRANCH-A');
  late _MemoryEndpointStore store;
  late ProviderContainer container;
  late Future<RatesModel> Function(String branchId) ratesHandler;

  ProviderContainer makeContainer() {
    store = _MemoryEndpointStore(endpointA);
    final c = ProviderContainer(
      overrides: [
        endpointStoreProvider.overrideWithValue(store),
        initialKioskEndpointProvider.overrideWithValue(endpointA),
        setupProbeProvider.overrideWithValue((e) async {
          return e.branchId == 'BRANCH-B' ? _branchBRates() : _branchARates();
        }),
        kioskL10nProvider.overrideWithValue(_l10n()),
        deviceLanguagesProvider.overrideWithValue(const ['en']),
        kioskRepositoryProvider.overrideWith(
          (ref) => _SwitchableBranchRepo(
            ref.watch(kioskBranchProvider),
            (b) => ratesHandler(b),
          ),
        ),
      ],
    );
    c.listen(kioskControllerProvider, (_, _) {});
    return c;
  }

  KioskState st() => container.read(kioskControllerProvider);
  KioskController kioskCtl() =>
      container.read(kioskControllerProvider.notifier);
  SetupController setupCtl() =>
      container.read(setupControllerProvider.notifier);

  void startBranchAWithReadySelection(FakeAsync async) {
    ratesHandler = (branchId) async =>
        branchId == 'BRANCH-B' ? _branchBRates() : _branchARates();
    container = makeContainer();
    kioskCtl().start();
    async.flushMicrotasks();

    kioskCtl()
      ..setCurrency('USD')
      ..setAmount('100')
      ..setPaymentMethod('Bank Transfer');

    expect(st().model?.branchName, 'Siam Branch A');
    expect(st().canReview, isTrue);
  }

  test('row 1: Setup Save to branch B while branch A\'s model is shown, before B '
      'answers -> No A rates / branch name shown; Review disabled', () {
    fakeAsync((async) {
      startBranchAWithReadySelection(async);

      final bRatesCompleter = Completer<RatesModel>();
      ratesHandler = (branchId) {
        if (branchId == 'BRANCH-B') return bRatesCompleter.future;
        return Future.value(_branchARates());
      };

      setupCtl().test('https://b.example.com', 'BRANCH-B');
      async.flushMicrotasks();
      setupCtl().save('https://b.example.com', 'BRANCH-B');
      async.flushMicrotasks();

      // Before B answers: A's model, rates, and currency must be gone and Review disabled
      expect(st().model, isNull);
      expect(st().currency, isNull);
      expect(st().estimate, isNull);
      expect(st().canReview, isFalse);

      bRatesCompleter.complete(_branchBRates());
      async.flushMicrotasks();
      container.dispose();
    });
  });

  test('row 2: Setup Save to branch B while branch A\'s model is shown, B\'s '
      'first read fails -> Error shown; no A model; Review disabled', () {
    fakeAsync((async) {
      startBranchAWithReadySelection(async);

      ratesHandler = (branchId) async {
        if (branchId == 'BRANCH-B') {
          throw const KioskFailure('Branch B offline');
        }
        return _branchARates();
      };

      setupCtl().test('https://b.example.com', 'BRANCH-B');
      async.flushMicrotasks();
      setupCtl().save('https://b.example.com', 'BRANCH-B');
      async.flushMicrotasks();

      expect(st().error, 'generic-en');
      expect(st().model, isNull);
      expect(st().currency, isNull);
      expect(st().canReview, isFalse);

      // Even if the customer re-enters amount and payment method, Review stays disabled
      kioskCtl()
        ..setAmount('100')
        ..setPaymentMethod('Cash');
      expect(st().canReview, isFalse);

      container.dispose();
    });
  });

  test(
    'row 3: Setup Save to branch B while branch A\'s model is shown, B '
    'answers -> B\'s rates and channels shown; Review enabled when valid',
    () {
      fakeAsync((async) {
        startBranchAWithReadySelection(async);

        final bRatesCompleter = Completer<RatesModel>();
        ratesHandler = (branchId) {
          if (branchId == 'BRANCH-B') return bRatesCompleter.future;
          return Future.value(_branchARates());
        };

        setupCtl().test('https://b.example.com', 'BRANCH-B');
        async.flushMicrotasks();
        setupCtl().save('https://b.example.com', 'BRANCH-B');
        async.flushMicrotasks();

        expect(st().model, isNull);
        expect(st().canReview, isFalse);

        bRatesCompleter.complete(_branchBRates());
        async.flushMicrotasks();

        expect(st().model?.branchName, 'Silom Branch B');
        expect(st().model?.rates.map((r) => r.code).toList(), ['EUR']);
        expect(st().currency, 'EUR');
        expect(st().availablePaymentMethods, ['Cash']);

        kioskCtl()
          ..setAmount('100')
          ..setPaymentMethod('Cash');
        expect(st().canReview, isTrue);

        container.dispose();
      });
    },
  );

  test('row 4 & edge case: Idle reset on the same endpoint keeps model without '
      'flicker, and resetGeneration bumps on every reset', () {
    fakeAsync((async) {
      startBranchAWithReadySelection(async);
      final genAfterStart = st().resetGeneration;

      // Same-endpoint idle reset with slow rates read: model stays non-null (no flicker)
      final slowRefresh = Completer<RatesModel>();
      ratesHandler = (_) => slowRefresh.future;

      kioskCtl().reset();
      async.flushMicrotasks();

      expect(st().model?.branchName, 'Siam Branch A');
      expect(st().currency, 'USD');
      expect(st().resetGeneration, genAfterStart + 1);

      slowRefresh.complete(_branchARates());
      async.flushMicrotasks();

      // Endpoint switch reset also bumps resetGeneration and drops the old model
      final slowB = Completer<RatesModel>();
      ratesHandler = (_) => slowB.future;

      setupCtl().test('https://b.example.com', 'BRANCH-B');
      async.flushMicrotasks();
      setupCtl().save('https://b.example.com', 'BRANCH-B');
      async.flushMicrotasks();

      expect(st().model, isNull);
      expect(st().resetGeneration, genAfterStart + 2);

      slowB.complete(_branchBRates());
      async.flushMicrotasks();
      container.dispose();
    });
  });
}
