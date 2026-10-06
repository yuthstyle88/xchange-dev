import 'dart:async';

/// Calls [poll] every [interval] while running. A call that is still in flight
/// when the next tick arrives is not doubled.
class StatusPoller {
  StatusPoller({required this.interval, required this.poll});

  final Duration interval;
  final Future<void> Function() poll;
  Timer? _timer;
  bool _inFlight = false;

  bool get isRunning => _timer != null;

  void start() {
    _timer ??= Timer.periodic(interval, (_) async {
      if (_inFlight) return;
      _inFlight = true;
      try {
        await poll();
      } finally {
        _inFlight = false;
      }
    });
  }

  void stop() {
    _timer?.cancel();
    _timer = null;
  }
}
