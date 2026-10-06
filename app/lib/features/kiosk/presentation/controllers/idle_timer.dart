import 'dart:async';

/// Fires [onIdle] after [timeout] without a [touch], then keeps watching.
class IdleTimer {
  IdleTimer({required this.onIdle, this.timeout = const Duration(minutes: 2)});

  final void Function() onIdle;
  final Duration timeout;
  Timer? _timer;

  void touch() {
    _timer?.cancel();
    _timer = Timer(timeout, () {
      onIdle();
      touch();
    });
  }

  void dispose() => _timer?.cancel();
}
