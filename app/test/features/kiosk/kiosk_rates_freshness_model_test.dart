import 'package:flutter_test/flutter_test.dart';
import 'package:xchange_kiosk/features/kiosk/domain/entities/kiosk_models.dart';

// RF-5 (#877, docs/design/blueprint/rates.md §4.10): the kiosk `rates` answer
// carries `freshness: {stale, sourceDate, publishedAt}`; the model keeps it.
// Written before the code by the ticket author (claude/dell-claude-4). Do not
// edit, delete or weaken these tests; add your own in another file.

Map<String, dynamic> answer([Map<String, dynamic>? freshness]) => {
  'branchId': 'MAIN',
  'branchName': 'XC-B',
  'open': true,
  'rates': [
    {'code': 'USD', 'nameEn': 'US Dollar', 'buy': 32.1, 'sell': 32.6},
  ],
  'simulation': false,
  'freshness': ?freshness,
};

void main() {
  test('the three facts are read', () {
    final m = RatesModel.fromJson(
      answer({
        'stale': true,
        'sourceDate': '2026-10-06',
        'publishedAt': '2026-10-10T02:00:00+00:00',
      }),
    );
    final f = m.freshness!;
    expect(f.stale, isTrue);
    expect(f.sourceDate, '2026-10-06');
    expect(f.publishedAt, DateTime.utc(2026, 10, 10, 2));
    expect(f.publishedAt!.isUtc, isTrue);
  });

  test('an offset publishedAt names the same instant', () {
    final m = RatesModel.fromJson(
      answer({
        'stale': false,
        'sourceDate': '2026-10-09',
        'publishedAt': '2026-10-10T09:00:00+07:00',
      }),
    );
    expect(m.freshness!.stale, isFalse);
    expect(m.freshness!.publishedAt, DateTime.utc(2026, 10, 10, 2));
  });

  test('nulls stay null', () {
    final m = RatesModel.fromJson(
      answer({'stale': false, 'sourceDate': null, 'publishedAt': null}),
    );
    expect(m.freshness, isNotNull);
    expect(m.freshness!.sourceDate, isNull);
    expect(m.freshness!.publishedAt, isNull);
  });

  test('a missing stale is not stale; an unreadable time is null', () {
    final f = RateFreshness.fromJson({
      'sourceDate': '2026-10-09',
      'publishedAt': 'yesterday',
    });
    expect(f.stale, isFalse);
    expect(f.sourceDate, '2026-10-09');
    expect(f.publishedAt, isNull);
  });

  test('the deployed v2 server sends no freshness: null', () {
    final m = RatesModel.fromJson(answer());
    expect(m.freshness, isNull);
    expect(m.rates.single.code, 'USD', reason: 'the rest is read as before');
  });
}
