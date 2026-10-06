import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../../l10n/kiosk_l10n.dart';
import '../../app/kiosk_providers.dart';
import '../../domain/entities/kiosk_models.dart';
import '../../domain/kiosk_failure.dart';
import '../../domain/repository/kiosk_repository.dart';
import 'idle_timer.dart';
import 'status_poller.dart';

/// The four customer screens.
enum KioskScreen { select, quote, waiting, receipt }

/// Payment methods in the order the v2 kiosk lists them.
const paymentMethods = [
  'Credit Card',
  'Alipay',
  'PromptPay',
  'Cash',
  'Bank Transfer',
];

/// When the shop pays out baht it does so in cash or by bank transfer only.
const buyPaymentMethods = ['Cash', 'Bank Transfer'];

/// What the customer would get for the typed amount, shown before the server
/// quotes. Display only — the figure that counts is the one in the quote.
class Estimate {
  const Estimate({required this.foreign, required this.receive});
  final double foreign, receive;
}

const _keep = Object();

class KioskState {
  const KioskState({
    required this.languageCode,
    this.screen = KioskScreen.select,
    this.model,
    this.type = ExchangeType.buy,
    this.currency,
    this.amountText = '',
    this.paymentMethod,
    this.quote,
    this.status,
    this.receiptImage,
    this.error = '',
    this.busy = false,
    this.quoteExpired = false,
    this.secondsLeft = 0,
  });

  final String languageCode;
  final KioskScreen screen;
  final RatesModel? model;
  final ExchangeType type;
  final String? currency;
  final String amountText;
  final String? paymentMethod;
  final QuoteModel? quote;
  final RequestIdentity? status;
  final String? receiptImage;

  /// Already localised; empty when there is nothing to say.
  final String error;
  final bool busy;
  final bool quoteExpired;
  final int secondsLeft;

  KioskState copyWith({
    String? languageCode,
    KioskScreen? screen,
    Object? model = _keep,
    ExchangeType? type,
    Object? currency = _keep,
    String? amountText,
    Object? paymentMethod = _keep,
    Object? quote = _keep,
    Object? status = _keep,
    Object? receiptImage = _keep,
    String? error,
    bool? busy,
    bool? quoteExpired,
    int? secondsLeft,
  }) => KioskState(
    languageCode: languageCode ?? this.languageCode,
    screen: screen ?? this.screen,
    model: identical(model, _keep) ? this.model : model as RatesModel?,
    type: type ?? this.type,
    currency: identical(currency, _keep) ? this.currency : currency as String?,
    amountText: amountText ?? this.amountText,
    paymentMethod: identical(paymentMethod, _keep)
        ? this.paymentMethod
        : paymentMethod as String?,
    quote: identical(quote, _keep) ? this.quote : quote as QuoteModel?,
    status: identical(status, _keep) ? this.status : status as RequestIdentity?,
    receiptImage: identical(receiptImage, _keep)
        ? this.receiptImage
        : receiptImage as String?,
    error: error ?? this.error,
    busy: busy ?? this.busy,
    quoteExpired: quoteExpired ?? this.quoteExpired,
    secondsLeft: secondsLeft ?? this.secondsLeft,
  );

  CurrencyRate? get selectedRate {
    for (final r in model?.rates ?? const <CurrencyRate>[]) {
      if (r.code == currency) return r;
    }
    return null;
  }

  List<String> get availablePaymentMethods =>
      type == ExchangeType.buy ? buyPaymentMethods : paymentMethods;

  Estimate? get estimate {
    final rate = selectedRate?.rateFor(type) ?? 0;
    final amount = double.tryParse(amountText.trim());
    if (amount == null || !amount.isFinite || amount <= 0 || amount > 1e7) {
      return null;
    }
    if (rate <= 0) return null;
    if (type == ExchangeType.buy) {
      final foreign = (amount * 100).round() / 100;
      return Estimate(
        foreign: foreign,
        receive: (foreign * rate * 100).round() / 100,
      );
    }
    final foreign = (amount / rate * 100).floor() / 100;
    return Estimate(foreign: foreign, receive: foreign);
  }

  bool get canReview {
    final e = estimate;
    return !busy &&
        (model?.open ?? false) &&
        e != null &&
        e.foreign > 0 &&
        paymentMethod != null;
  }

  bool get canConfirm =>
      !busy && quote != null && secondsLeft <= 0 && !quoteExpired;

  RequestIdentity? get identity => status ?? quote?.identity;
}

class KioskController extends Notifier<KioskState> {
  static const reviewSeconds = 3;
  static const restartAfterDone = Duration(seconds: 30);

  late final KioskRepository _repo = ref.read(kioskRepositoryProvider);
  late final KioskL10n _l10n = ref.read(kioskL10nProvider);

  /// The language the screen returns to on every reset.
  late final String defaultLanguage = _l10n.detect(
    ref.read(deviceLanguagesProvider),
  );

  late final IdleTimer _idle = IdleTimer(onIdle: _onIdle);
  late final StatusPoller _statusPoller = StatusPoller(
    interval: const Duration(seconds: 5),
    poll: _pollStatus,
  );
  late final StatusPoller _receiptPoller = StatusPoller(
    interval: const Duration(seconds: 3),
    poll: _pollReceipt,
  );
  Timer? _countdown;
  Timer? _restart;
  bool _disposed = false;

  KioskTexts get _t => _l10n.of(state.languageCode);

  @override
  KioskState build() {
    ref.onDispose(() {
      _disposed = true;
      _countdown?.cancel();
      _restart?.cancel();
      _statusPoller.stop();
      _receiptPoller.stop();
      _idle.dispose();
    });
    return KioskState(languageCode: defaultLanguage);
  }

  void _set(KioskState next) {
    if (!_disposed) state = next;
  }

  String _failure(Object e) =>
      e is KioskFailure ? _t.errorText(e.serverMessage) : _t.text('errGeneric');

  void reportError(String textKey) {
    _set(state.copyWith(error: _t.text(textKey)));
  }

  // ---- lifecycle ---------------------------------------------------------

  /// Loads the rates and returns the screen to its starting state.
  Future<void> start() => reset();

  Future<void> reset() async {
    _countdown?.cancel();
    _restart?.cancel();
    _restart = null;
    _statusPoller.stop();
    _receiptPoller.stop();
    _idle.touch();
    _set(
      KioskState(
        languageCode: defaultLanguage,
        model: state.model,
        currency: state.currency,
      ),
    );
    try {
      final m = await _repo.rates();
      if (_disposed) return;
      final keep = m.rates.any((r) => r.code == state.currency);
      _set(
        state.copyWith(
          model: m,
          currency: keep
              ? state.currency
              : (m.rates.isEmpty ? null : m.rates.first.code),
          error: m.open ? '' : _t.text('closed'),
        ),
      );
    } catch (e) {
      _set(state.copyWith(error: _failure(e)));
    }
  }

  /// Any tap or key press on the kiosk.
  void activity() => _idle.touch();

  void _onIdle() {
    final atCounter =
        state.screen == KioskScreen.waiting ||
        state.screen == KioskScreen.receipt;
    if (!state.busy && !atCounter) reset();
  }

  // ---- screen 1: select --------------------------------------------------

  void setLanguage(String code) {
    final t = _l10n.of(code);
    final closed =
        state.model != null &&
        !state.model!.open &&
        state.screen == KioskScreen.select;
    _set(
      state.copyWith(
        languageCode: t.code,
        error: closed
            ? t.text('closed')
            : state.quoteExpired
            ? t.text('quoteExpired')
            : '',
      ),
    );
  }

  void setType(ExchangeType value) {
    final allowed = value == ExchangeType.buy
        ? buyPaymentMethods
        : paymentMethods;
    _set(
      state.copyWith(
        type: value,
        paymentMethod: allowed.contains(state.paymentMethod)
            ? state.paymentMethod
            : null,
      ),
    );
  }

  void setCurrency(String code) => _set(state.copyWith(currency: code));

  void setAmount(String text) => _set(state.copyWith(amountText: text));

  void setPaymentMethod(String? method) =>
      _set(state.copyWith(paymentMethod: method));

  Future<void> submitQuote() async {
    if (!state.canReview) return;
    _set(state.copyWith(busy: true, error: ''));
    try {
      final quote = await _repo.quote(
        type: state.type,
        currency: state.currency!,
        amount: state.amountText.trim(),
        paymentMethod: state.paymentMethod!,
      );
      if (_disposed) return;
      _set(
        state.copyWith(
          quote: quote,
          status: null,
          quoteExpired: false,
          screen: KioskScreen.quote,
          secondsLeft: reviewSeconds,
          busy: false,
        ),
      );
      _countdown?.cancel();
      _countdown = Timer.periodic(const Duration(seconds: 1), (t) {
        final left = state.secondsLeft - 1;
        if (left <= 0) t.cancel();
        _set(state.copyWith(secondsLeft: left));
      });
      _statusPoller.start();
    } catch (e) {
      _set(state.copyWith(busy: false, error: _failure(e)));
    }
  }

  // ---- screen 2: quote ---------------------------------------------------

  void backToSelect() {
    _countdown?.cancel();
    _statusPoller.stop();
    _set(
      state.copyWith(
        quote: null,
        status: null,
        quoteExpired: false,
        secondsLeft: 0,
        error: '',
        screen: KioskScreen.select,
      ),
    );
  }

  Future<void> confirm() async {
    final q = state.quote;
    if (!state.canConfirm || q == null) return;
    _set(state.copyWith(busy: true, error: ''));
    try {
      final status = await _repo.confirm(q.id);
      if (_disposed) return;
      _set(
        state.copyWith(
          status: status,
          screen: KioskScreen.waiting,
          busy: false,
        ),
      );
      _receiptPoller.start();
      _afterStatus();
    } catch (e) {
      _set(state.copyWith(busy: false, error: _failure(e)));
    }
  }

  // ---- screens 3 and 4: waiting, receipt ---------------------------------

  Future<void> _pollStatus() async {
    final q = state.quote;
    if (q == null) return;
    try {
      final s = await _repo.status(q.id);
      if (_disposed || state.quote?.id != q.id) return;
      final expired =
          s.status == KioskStatus.expired && state.screen == KioskScreen.quote;
      if (expired) _countdown?.cancel();
      _set(
        state.copyWith(
          status: s,
          quoteExpired: state.quoteExpired || expired,
          error: expired ? _t.text('quoteExpired') : '',
        ),
      );
      _afterStatus();
    } catch (_) {
      if (state.quote?.id == q.id) {
        _set(state.copyWith(error: _t.text('statusError')));
      }
    }
  }

  Future<void> _pollReceipt() async {
    final q = state.quote;
    if (q == null) return;
    try {
      final r = await _repo.customerReceipt(q.id);
      if (_disposed || state.quote?.id != q.id) return;
      _set(
        state.copyWith(
          receiptImage: r.image,
          status: r.identity,
          screen: r.image == null ? KioskScreen.waiting : KioskScreen.receipt,
        ),
      );
      _afterStatus();
    } catch (_) {
      // Keep the last image through a brief connection failure.
    }
  }

  void _afterStatus() {
    if (state.status?.status == KioskStatus.completed && _restart == null) {
      final id = state.quote?.id;
      _restart = Timer(restartAfterDone, () {
        if (state.quote?.id == id) reset();
      });
    }
  }
}
