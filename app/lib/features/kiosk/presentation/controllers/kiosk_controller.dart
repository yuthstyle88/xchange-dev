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

/// [typed] as the server reads a number: a digit on each side of the dot.
/// The amount box accepts `100.` and `.5` while the customer is typing; v2
/// read them with JavaScript's `Number`, v3 refuses them (design §3, #67).
String plainDecimal(String typed) {
  var t = typed.trim();
  if (t.endsWith('.')) t = t.substring(0, t.length - 1);
  if (t.startsWith('.')) t = '0$t';
  return t;
}

final _decimalPattern = RegExp(r'^(\d+)(?:\.(\d+))?(?:[eE]([+-]?\d+))?$');

BigInt _tenPow(int n) => BigInt.from(10).pow(n);

({BigInt digits, int scale})? _scaled(String raw) {
  final m = _decimalPattern.firstMatch(plainDecimal(raw));
  if (m == null) return null;
  final whole = m.group(1)!;
  final frac = m.group(2) ?? '';
  final exp = int.tryParse(m.group(3) ?? '0');
  if (exp == null) return null;
  final digits = BigInt.tryParse('$whole$frac');
  if (digits == null) return null;
  final scale = frac.length - exp;
  if (scale < 0) {
    return (digits: digits * _tenPow(-scale), scale: 0);
  }
  return (digits: digits, scale: scale);
}

double _roundHalfUp2(BigInt num, BigInt den) {
  final scaled = num * BigInt.from(100);
  var cents = scaled ~/ den;
  if (scaled.remainder(den) * BigInt.two >= den) {
    cents += BigInt.one;
  }
  return cents.toDouble() / 100;
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
    this.resetGeneration = 0,
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

  /// Bumped by every [KioskController.reset], so screen-local inputs (the
  /// currency search) can clear even when the screen is not rebuilt.
  final int resetGeneration;

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
    resetGeneration: resetGeneration,
  );

  CurrencyRate? get selectedRate {
    for (final r in model?.rates ?? const <CurrencyRate>[]) {
      if (r.code == currency) return r;
    }
    return null;
  }

  List<String> get availablePaymentMethods => offeredPaymentMethods(type);

  /// The channels offered for [direction]: with the branch's channels, those
  /// on for it, in the kiosk's order (v2 `kiosk.js:42`; a channel with no row
  /// is not offered); without them, the fixed lists.
  List<String> offeredPaymentMethods(ExchangeType direction) {
    final channels = model?.paymentMethods;
    if (channels == null) {
      return direction == ExchangeType.buy ? buyPaymentMethods : paymentMethods;
    }
    return [
      for (final id in paymentMethods)
        if (channels.any((c) => c.id == id && c.offeredFor(direction))) id,
    ];
  }

  /// The chosen channel's instruction for the customer, '' when there is none.
  String get paymentInstruction {
    for (final c in model?.paymentMethods ?? const <PaymentChannel>[]) {
      if (c.id == paymentMethod) return c.instruction;
    }
    return '';
  }

  Estimate? get estimate {
    final rate = selectedRate?.rateFor(type) ?? 0;
    if (!rate.isFinite || rate <= 0) return null;
    final a = _scaled(amountText);
    final r = _scaled(rate.toString());
    if (a == null ||
        r == null ||
        a.digits <= BigInt.zero ||
        r.digits <= BigInt.zero ||
        a.digits > BigInt.from(10000000) * _tenPow(a.scale)) {
      return null;
    }
    if (type == ExchangeType.buy) {
      return Estimate(
        foreign: _roundHalfUp2(a.digits, _tenPow(a.scale)),
        receive: _roundHalfUp2(a.digits * r.digits, _tenPow(a.scale + r.scale)),
      );
    }
    final cents =
        (a.digits * _tenPow(r.scale + 2)) ~/ (r.digits * _tenPow(a.scale));
    final foreign = cents.toDouble() / 100;
    return Estimate(foreign: foreign, receive: foreign);
  }

  bool get canReview {
    final e = estimate;
    return !busy &&
        (model?.open ?? false) &&
        e != null &&
        e.foreign > 0 &&
        paymentMethod != null &&
        availablePaymentMethods.contains(paymentMethod);
  }

  bool get canConfirm =>
      !busy && quote != null && secondsLeft <= 0 && !quoteExpired;

  RequestIdentity? get identity => status ?? quote?.identity;
}

class KioskController extends Notifier<KioskState> {
  static const reviewSeconds = 3;
  static const restartAfterDone = Duration(seconds: 30);
  static const ratesRefresh = Duration(seconds: 30);

  KioskRepository get _repo => ref.read(kioskRepositoryProvider);
  late final KioskL10n _l10n = ref.read(kioskL10nProvider);

  /// The language the screen returns to on every reset.
  late final String defaultLanguage = _l10n.detect(
    ref.read(deviceLanguagesProvider),
  );

  late final IdleTimer _idle = IdleTimer(onIdle: _onIdle);
  late final StatusPoller _ratesPoller = StatusPoller(
    interval: ratesRefresh,
    poll: _refreshRates,
  );
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
  bool _ratesError = false;
  int _ratesSeq = 0;
  int _ratesMinSeq = 0;
  String? _errorKey;
  String? _errorServerMessage;

  KioskTexts get _t => _l10n.of(state.languageCode);

  @override
  KioskState build() {
    ref.onDispose(() {
      _disposed = true;
      _countdown?.cancel();
      _restart?.cancel();
      _ratesPoller.stop();
      _statusPoller.stop();
      _receiptPoller.stop();
      _idle.dispose();
    });
    return KioskState(languageCode: defaultLanguage);
  }

  void _set(KioskState next) {
    if (!_disposed) state = next;
  }

  String _noError() {
    _errorKey = null;
    _errorServerMessage = null;
    return '';
  }

  String _keyError(String key, [KioskTexts? t]) {
    _errorKey = key;
    _errorServerMessage = null;
    return (t ?? _t).text(key);
  }

  String _failure(Object e) {
    if (e is KioskFailure) {
      _errorKey = null;
      _errorServerMessage = e.serverMessage;
      return _t.errorText(e.serverMessage);
    }
    return _keyError('errGeneric');
  }

  String _currentError(KioskTexts t) {
    if (_errorServerMessage != null) {
      return t.errorText(_errorServerMessage!);
    }
    if (_errorKey != null) {
      return t.text(_errorKey!);
    }
    return '';
  }

  void reportError(String textKey) {
    _ratesError = false;
    _set(state.copyWith(error: _keyError(textKey)));
  }

  // ---- lifecycle ---------------------------------------------------------

  /// Loads the rates and returns the screen to its starting state.
  Future<void> start() => reset();

  Future<void> reset() async {
    _countdown?.cancel();
    _restart?.cancel();
    _restart = null;
    _ratesPoller.start();
    _statusPoller.stop();
    _receiptPoller.stop();
    _idle.touch();
    _ratesError = false;
    _noError();
    final seq = ++_ratesSeq;
    _ratesMinSeq = seq;
    _set(
      KioskState(
        languageCode: defaultLanguage,
        model: state.model,
        currency: state.currency,
        resetGeneration: state.resetGeneration + 1,
      ),
    );
    try {
      final m = await _repo.rates();
      if (_disposed || seq < _ratesMinSeq) return;
      _ratesMinSeq = seq;
      _ratesError = false;
      _applyRates(m, error: m.open ? _noError() : _keyError('closed'));
    } catch (e) {
      if (_disposed || seq < _ratesMinSeq) return;
      _ratesMinSeq = seq;
      _ratesError = true;
      _set(state.copyWith(error: _failure(e)));
    }
  }

  void _applyRates(RatesModel m, {required String error}) {
    final keep = m.rates.any((r) => r.code == state.currency);
    final currency = keep
        ? state.currency
        : (m.rates.isEmpty ? null : m.rates.first.code);
    final amountText = keep ? state.amountText : '';
    _set(
      _keepOfferedMethod(
        state.copyWith(
          model: m,
          currency: currency,
          amountText: amountText,
          error: error,
        ),
      ),
    );
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

  Future<void> _refreshRates() async {
    if (state.screen != KioskScreen.select ||
        state.busy ||
        state.quote != null) {
      return;
    }
    final seq = ++_ratesSeq;
    try {
      final m = await _repo.rates();
      if (_disposed ||
          seq < _ratesMinSeq ||
          state.screen != KioskScreen.select ||
          state.busy ||
          state.quote != null) {
        return;
      }
      _ratesMinSeq = seq;
      final openChanged =
          state.model == null || m.open != state.model!.open || _ratesError;
      _ratesError = false;
      final error = openChanged
          ? (m.open ? _noError() : _keyError('closed'))
          : state.error;
      _applyRates(m, error: error);
    } catch (_) {}
  }

  void setLanguage(String code) {
    final t = _l10n.of(code);
    _set(state.copyWith(languageCode: t.code, error: _currentError(t)));
  }

  void setType(ExchangeType value) {
    final allowed = state.offeredPaymentMethods(value);
    _set(
      state.copyWith(
        type: value,
        paymentMethod: allowed.contains(state.paymentMethod)
            ? state.paymentMethod
            : null,
      ),
    );
  }

  /// [next] with its chosen channel cleared when the channels it now has do
  /// not offer it (v2 `kiosk.js:42`).
  KioskState _keepOfferedMethod(KioskState next) =>
      next.availablePaymentMethods.contains(next.paymentMethod)
      ? next
      : next.copyWith(paymentMethod: null);

  void setCurrency(String code) => _set(state.copyWith(currency: code));

  void setAmount(String text) => _set(state.copyWith(amountText: text));

  void setPaymentMethod(String? method) =>
      _set(state.copyWith(paymentMethod: method));

  Future<void> submitQuote() async {
    if (!state.canReview) return;
    _ratesError = false;
    _set(state.copyWith(busy: true, error: _noError()));
    try {
      final quote = await _repo.quote(
        type: state.type,
        currency: state.currency!,
        amount: plainDecimal(state.amountText),
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
      _ratesError = false;
      _set(state.copyWith(busy: false, error: _failure(e)));
    }
  }

  // ---- screen 2: quote ---------------------------------------------------

  void backToSelect() {
    _countdown?.cancel();
    _statusPoller.stop();
    _ratesError = false;
    _set(
      state.copyWith(
        quote: null,
        status: null,
        quoteExpired: false,
        secondsLeft: 0,
        error: _noError(),
        screen: KioskScreen.select,
      ),
    );
  }

  Future<void> confirm() async {
    final q = state.quote;
    if (!state.canConfirm || q == null) return;
    _ratesError = false;
    _set(state.copyWith(busy: true, error: _noError()));
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
      if (state.screen != KioskScreen.quote || state.quote?.id != q.id) {
        _set(state.copyWith(busy: false));
        return;
      }
      _ratesError = false;
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
      final confirmed =
          state.screen == KioskScreen.quote &&
          (s.status == KioskStatus.waiting ||
              s.status == KioskStatus.accepted ||
              s.status == KioskStatus.idChecked ||
              s.status == KioskStatus.completed);
      if (confirmed) {
        _countdown?.cancel();
        _set(
          state.copyWith(
            status: s,
            screen: KioskScreen.waiting,
            busy: false,
            error: _noError(),
          ),
        );
        _receiptPoller.start();
        _afterStatus();
        return;
      }
      final expired =
          s.status == KioskStatus.expired && state.screen == KioskScreen.quote;
      if (expired) _countdown?.cancel();
      _set(
        state.copyWith(
          status: s,
          quoteExpired: state.quoteExpired || expired,
          error: expired
              ? _keyError('quoteExpired')
              : _errorKey == 'statusError'
              ? _noError()
              : state.error,
        ),
      );
      _afterStatus();
    } catch (_) {
      if (state.quote?.id == q.id) {
        _set(state.copyWith(error: _keyError('statusError')));
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
