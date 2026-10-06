enum EndpointProblem {
  serverMissing,
  serverInvalid,
  serverNotSecure,
  branchMissing,
  branchInvalid,
}

class EndpointFormatException implements Exception {
  const EndpointFormatException(this.problem);
  final EndpointProblem problem;

  @override
  String toString() => 'EndpointFormatException: $problem';
}

class KioskEndpoint {
  const KioskEndpoint._(this.serverOrigin, this.branchId);

  final Uri serverOrigin;
  final String branchId;

  String get origin {
    if (serverOrigin.hasPort) {
      final isDefaultPort =
          (serverOrigin.scheme == 'https' && serverOrigin.port == 443) ||
          (serverOrigin.scheme == 'http' && serverOrigin.port == 80);
      if (!isDefaultPort) {
        return '${serverOrigin.scheme}://${serverOrigin.host}:${serverOrigin.port}';
      }
    }
    return '${serverOrigin.scheme}://${serverOrigin.host}';
  }

  factory KioskEndpoint.parse(
    String server,
    String branch, {
    bool allowInsecure = false,
  }) {
    final trimmedServer = server.trim();
    if (trimmedServer.isEmpty) {
      throw const EndpointFormatException(EndpointProblem.serverMissing);
    }

    if (trimmedServer.contains(' ') || trimmedServer.startsWith('://')) {
      throw const EndpointFormatException(EndpointProblem.serverInvalid);
    }

    final String rawWithScheme;
    if (trimmedServer.contains('://')) {
      rawWithScheme = trimmedServer;
    } else {
      rawWithScheme = 'https://$trimmedServer';
    }

    final Uri uri;
    try {
      uri = Uri.parse(rawWithScheme);
    } on FormatException {
      throw const EndpointFormatException(EndpointProblem.serverInvalid);
    }

    final scheme = uri.scheme.toLowerCase();
    if (scheme != 'https' && scheme != 'http') {
      throw const EndpointFormatException(EndpointProblem.serverInvalid);
    }

    if (uri.host.isEmpty || uri.userInfo.isNotEmpty || uri.host.contains(' ')) {
      throw const EndpointFormatException(EndpointProblem.serverInvalid);
    }

    if (scheme == 'http' && !allowInsecure) {
      throw const EndpointFormatException(EndpointProblem.serverNotSecure);
    }

    final host = uri.host.toLowerCase();
    int? port = uri.hasPort ? uri.port : null;
    if ((scheme == 'https' && port == 443) ||
        (scheme == 'http' && port == 80)) {
      port = null;
    }

    final serverOrigin = port != null
        ? Uri(scheme: scheme, host: host, port: port)
        : Uri(scheme: scheme, host: host);

    final trimmedBranch = branch.trim();
    if (trimmedBranch.isEmpty) {
      throw const EndpointFormatException(EndpointProblem.branchMissing);
    }

    if (trimmedBranch.contains(RegExp(r'\s')) || trimmedBranch.length > 80) {
      throw const EndpointFormatException(EndpointProblem.branchInvalid);
    }

    return KioskEndpoint._(serverOrigin, trimmedBranch);
  }

  @override
  bool operator ==(Object other) =>
      identical(this, other) ||
      other is KioskEndpoint &&
          runtimeType == other.runtimeType &&
          origin == other.origin &&
          branchId == other.branchId;

  @override
  int get hashCode => Object.hash(origin, branchId);

  @override
  String toString() => 'KioskEndpoint(origin: $origin, branchId: $branchId)';
}
