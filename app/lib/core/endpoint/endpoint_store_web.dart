import 'endpoint_store.dart';
import 'kiosk_endpoint.dart';

class PageEndpointStore implements EndpointStore {
  const PageEndpointStore();

  @override
  Future<KioskEndpoint?> load() async {
    final branchParam = Uri.base.queryParameters['branch']?.trim();
    final branch = (branchParam != null && branchParam.isNotEmpty)
        ? branchParam
        : 'MAIN';
    final origin = Uri.base.origin;
    return KioskEndpoint.parse(origin, branch, allowInsecure: true);
  }

  @override
  Future<void> save(KioskEndpoint endpoint) async {}

  @override
  Future<void> clear() async {}

  @override
  String? originHeader(KioskEndpoint endpoint) => null;
}

EndpointStore createEndpointStore() => const PageEndpointStore();
