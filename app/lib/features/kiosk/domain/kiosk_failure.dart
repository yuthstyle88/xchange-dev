/// The server refused a kiosk request. [serverMessage] is its `error` text —
/// the screens map it to a localised sentence and never show it as sent.
/// Empty when there was no answer to read (offline, timeout, malformed).
class KioskFailure implements Exception {
  const KioskFailure(this.serverMessage);
  final String serverMessage;

  @override
  String toString() => 'KioskFailure($serverMessage)';
}
