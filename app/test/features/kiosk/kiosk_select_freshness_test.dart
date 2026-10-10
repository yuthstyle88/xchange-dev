import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/features/kiosk/app/kiosk_providers.dart';
import 'package:xchange_kiosk/features/kiosk/domain/entities/kiosk_models.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/controllers/kiosk_controller.dart';
import 'package:xchange_kiosk/features/kiosk/presentation/pages/kiosk_page.dart';
import 'package:xchange_kiosk/l10n/kiosk_l10n.dart';

import 'kiosk_controller_test.dart' show FakeRepository;

// RF-5 (#877, docs/design/blueprint/rates.md §4.8, §4.10): the select page
// shows the stale badge and, separately, the reference date and the publish
// time (Bangkok). It never says "real time" or "live".
// Written before the code by the ticket author (claude/dell-claude-4). Do not
// edit, delete or weaken these tests; add your own in another file.
//
// Built from the JSON the server sends (RatesModel.fromJson), so the tests
// compile on main and fail on what the page shows.

/// The fake server, answering `rates` with [freshness] (absent when null).
class FreshServer extends FakeRepository {
  FreshServer(this.freshness);
  final Map<String, dynamic>? freshness;

  @override
  Future<RatesModel> rates() async {
    calls.add('rates');
    return RatesModel.fromJson({
      'branchId': 'MAIN',
      'branchName': 'XC-B',
      'open': true,
      'rates': [
        {'code': 'USD', 'nameEn': 'US Dollar', 'buy': 32.1, 'sell': 32.6},
      ],
      'simulation': false,
      'freshness': ?freshness,
    });
  }
}

/// Test texts: the four RF-5 keys in English, everything else its key.
KioskL10n testTexts() => KioskL10n.fromJson({
  'langs': [
    {'code': 'en', 'lang': 'en', 'name': 'English'},
  ],
  'text': {
    'en': {
      'rateStale': 'Reference out of date',
      'rateSourceDate': 'Reference date {date}',
      'rateNoSourceDate': 'No reference',
      'ratePublishedAt': 'Published {at}',
    },
  },
  'errors': <String, String>{},
});

/// The kiosk's real texts, read from the repository.
KioskL10n assetTexts() => KioskL10n.fromJson(
  jsonDecode(File(KioskL10n.assetPath).readAsStringSync())
      as Map<String, dynamic>,
);

final forbidden = RegExp(
  r'real\s*-?\s*time|\blive\b|เรียลไทม์',
  caseSensitive: false,
);

void main() {
  late ProviderContainer container;

  Future<void> pumpSelect(
    WidgetTester tester,
    Map<String, dynamic>? freshness, {
    KioskL10n? texts,
    String language = 'en',
  }) async {
    tester.view.physicalSize = const Size(1280, 1400);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);
    container = ProviderContainer(
      overrides: [
        kioskRepositoryProvider.overrideWithValue(FreshServer(freshness)),
        kioskL10nProvider.overrideWithValue(texts ?? testTexts()),
        deviceLanguagesProvider.overrideWithValue([language]),
      ],
    );
    await tester.pumpWidget(
      UncontrolledProviderScope(
        container: container,
        child: const MaterialApp(home: KioskPage()),
      ),
    );
    await container.read(kioskControllerProvider.notifier).start();
    await tester.pump();
    expect(container.read(kioskControllerProvider).screen, KioskScreen.select);
  }

  Future<void> finish(WidgetTester tester) async {
    await tester.pumpWidget(const SizedBox());
    container.dispose();
  }

  Finder block() => find.byKey(const Key('rate-freshness'));
  Finder inBlock(Finder f) => find.descendant(of: block(), matching: f);

  /// Every text drawn on the page.
  List<String> drawnTexts(WidgetTester tester) => [
    for (final t in tester.widgetList<Text>(find.byType(Text)))
      t.data ?? t.textSpan?.toPlainText() ?? '',
    for (final r in tester.widgetList<RichText>(find.byType(RichText)))
      r.text.toPlainText(),
  ];

  testWidgets('a stale publication shows the badge and both dates', (
    tester,
  ) async {
    await pumpSelect(tester, {
      'stale': true,
      'sourceDate': '2026-10-06',
      'publishedAt': '2026-10-10T02:00:00+00:00',
    });
    expect(block(), findsOneWidget);
    final badge = find.byKey(const Key('rate-stale'));
    expect(badge, findsOneWidget);
    expect(
      find.descendant(of: badge, matching: find.text('Reference out of date')),
      findsOneWidget,
    );
    expect(
      inBlock(find.textContaining('Reference date 2026-10-06')),
      findsOneWidget,
    );
    expect(
      inBlock(find.textContaining('Published 2026-10-10 09:00')),
      findsOneWidget,
      reason: '02:00 UTC is 09:00 in Bangkok',
    );
    await finish(tester);
  });

  testWidgets('a fresh publication shows both dates and no badge', (
    tester,
  ) async {
    await pumpSelect(tester, {
      'stale': false,
      'sourceDate': '2026-10-09',
      'publishedAt': '2026-10-10T02:00:00+00:00',
    });
    expect(block(), findsOneWidget);
    expect(find.byKey(const Key('rate-stale')), findsNothing);
    expect(find.text('Reference out of date'), findsNothing);
    expect(
      inBlock(find.textContaining('Reference date 2026-10-09')),
      findsOneWidget,
    );
    expect(
      inBlock(find.textContaining('Published 2026-10-10 09:00')),
      findsOneWidget,
    );
    await finish(tester);
  });

  testWidgets(
    'the publish time is Bangkok time: past 17:00 UTC is the next day',
    (tester) async {
      await pumpSelect(tester, {
        'stale': false,
        'sourceDate': '2026-10-09',
        'publishedAt': '2026-10-09T20:30:00Z',
      });
      expect(
        inBlock(find.textContaining('Published 2026-10-10 03:30')),
        findsOneWidget,
      );
      await finish(tester);
    },
  );

  testWidgets('no reference and no publication: the no-reference text only', (
    tester,
  ) async {
    await pumpSelect(tester, {
      'stale': false,
      'sourceDate': null,
      'publishedAt': null,
    });
    expect(block(), findsOneWidget);
    expect(inBlock(find.textContaining('No reference')), findsOneWidget);
    expect(find.textContaining('Reference date'), findsNothing);
    expect(find.textContaining('Published'), findsNothing);
    expect(find.byKey(const Key('rate-stale')), findsNothing);
    await finish(tester);
  });

  testWidgets('the deployed v2 server sends no freshness: no block', (
    tester,
  ) async {
    await pumpSelect(tester, null);
    expect(block(), findsNothing);
    expect(find.byKey(const Key('rate-stale')), findsNothing);
    await finish(tester);
  });

  for (final language in ['en', 'th']) {
    testWidgets(
      'with the real $language texts the page never says real time or live',
      (tester) async {
        await pumpSelect(
          tester,
          {
            'stale': true,
            'sourceDate': '2026-10-06',
            'publishedAt': '2026-10-10T02:00:00+00:00',
          },
          texts: assetTexts(),
          language: language,
        );
        expect(find.byKey(const Key('rate-stale')), findsOneWidget);
        final texts = inBlock(find.byType(Text));
        expect(
          texts,
          findsAtLeastNWidgets(2),
          reason: 'the badge and the dates are drawn',
        );
        for (final t in tester.widgetList<Text>(texts)) {
          final s = t.data ?? t.textSpan?.toPlainText() ?? '';
          expect(s, isNot(contains('rateStale')), reason: 'a key, not a text');
          expect(s, isNot(contains('rateSourceDate')));
          expect(s, isNot(contains('ratePublishedAt')));
          expect(s, isNot(contains('rateNoSourceDate')));
        }
        final said = drawnTexts(tester).where(forbidden.hasMatch).toList();
        expect(said, isEmpty, reason: 'no display says real time (§4.8)');
        await finish(tester);
      },
    );
  }
}
