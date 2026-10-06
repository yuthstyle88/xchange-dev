import 'package:shared_preferences/shared_preferences.dart';

import 'endpoint_store.dart';
import 'kiosk_endpoint.dart';

class DeviceEndpointStore implements EndpointStore {
  const DeviceEndpointStore();

  static const String serverKey = 'kiosk.server';
  static const String branchKey = 'kiosk.branch';

  @override
  Future<KioskEndpoint?> load() async {
    final prefs = await SharedPreferences.getInstance();
    final server = prefs.getString(serverKey);
    final branch = prefs.getString(branchKey);
    if (server == null || branch == null) return null;
    try {
      return KioskEndpoint.parse(server, branch, allowInsecure: true);
    } on EndpointFormatException {
      return null;
    }
  }

  @override
  Future<void> save(KioskEndpoint endpoint) async {
    final prefs = await SharedPreferences.getInstance();
    await prefs.setString(serverKey, endpoint.origin);
    await prefs.setString(branchKey, endpoint.branchId);
  }

  @override
  Future<void> clear() async {
    final prefs = await SharedPreferences.getInstance();
    await prefs.remove(serverKey);
    await prefs.remove(branchKey);
  }

  @override
  String? originHeader(KioskEndpoint endpoint) => endpoint.origin;
}

EndpointStore createEndpointStore() => const DeviceEndpointStore();
