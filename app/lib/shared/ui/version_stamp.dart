import 'package:dio/dio.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../core/network/dio_provider.dart';

/// One-shot GET /health/ready through the app's existing dio client.
final serverVersionStampProvider = FutureProvider<String>((ref) async {
  try {
    Dio dio;
    try {
      dio = ref.watch(dioProvider);
    } catch (_) {
      return 'server ?';
    }
    final res = await dio.get<dynamic>('/health/ready');
    if (res.statusCode == 200 && res.data is Map) {
      final data = res.data as Map;
      final v = data['version']?.toString().trim();
      final b = data['build']?.toString().trim();
      if (v != null && v.isNotEmpty && b != null && b.isNotEmpty) {
        final shortB = b.length > 7 ? b.substring(0, 7) : b;
        return 'server v$v · $shortB';
      }
    }
  } catch (_) {
    // If that fails, show server ? and do not retry in a loop.
  }
  return 'server ?';
});

class VersionStamp extends ConsumerWidget {
  const VersionStamp({
    super.key,
    this.appVersion,
    this.buildSha,
    this.builtAt,
    this.channel,
  });

  final String? appVersion;
  final String? buildSha;
  final String? builtAt;
  final String? channel;

  static const defaultAppVersion = String.fromEnvironment(
    'APP_VERSION',
    defaultValue: 'unknown',
  );
  static const defaultBuildSha = String.fromEnvironment(
    'BUILD_SHA',
    defaultValue: 'unknown',
  );
  static const defaultBuiltAt = String.fromEnvironment(
    'BUILT_AT',
    defaultValue: 'unknown',
  );
  static const defaultChannel = String.fromEnvironment(
    'CHANNEL',
    defaultValue: 'unknown',
  );

  String _formatApp() {
    final v = (appVersion ?? defaultAppVersion).trim();
    final ver = v.isEmpty ? 'unknown' : v;

    final s = (buildSha ?? defaultBuildSha).trim();
    final fullSha = s.isEmpty ? 'unknown' : s;
    final sha = fullSha.length > 7 ? fullSha.substring(0, 7) : fullSha;

    final c = (channel ?? defaultChannel).trim();
    final chan = c.isEmpty ? 'unknown' : c;

    return 'v$ver · $sha · $chan';
  }

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    // Keep reference to BUILT_AT define per versioning standard.
    final _ = (builtAt ?? defaultBuiltAt).trim();

    final appPart = _formatApp();
    final serverPart = ref
        .watch(serverVersionStampProvider)
        .maybeWhen(data: (val) => val, orElse: () => 'server ?');

    final fullText = '$appPart · $serverPart';

    return Text(
      fullText,
      key: const Key('version-stamp'),
      textAlign: TextAlign.center,
      style:
          Theme.of(context).textTheme.bodySmall?.copyWith(
            color: Theme.of(context).colorScheme.outline,
            fontSize: 11,
          ) ??
          const TextStyle(color: Colors.grey, fontSize: 11),
    );
  }
}
