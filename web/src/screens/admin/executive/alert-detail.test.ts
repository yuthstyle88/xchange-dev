// OL-7 row 64 (verification of 2026-11-07): an executive alert's line printed the server's English sentence with
// record ids in it. The API now sends figures only (staleMinutes, differences, stock; no `detail`), and
// `alertDetail(a)` in ./alert-detail.ts words them in the screen's language:
// - ratePending, or a countDifference with no figures (an undecodable row): '' (the row shows kind, branch, time);
// - staleData: alertStaleMinutes with {n};
// - countDifference: "THB 12.50 · USD -3.00", each currency and its amount, in the order sent;
// - lowStock: alertStock with {currency} {available} {floor} {onHand} {reserved}.
// Written before the fix by claude/bbe9ce8b for the OL sub-controller claude/1cc51615. Do not edit, delete or weaken.
import { afterEach, describe, expect, it } from 'vitest';
import { setLang } from '../../../shared/i18n';
import { makeAlertDto } from '../../../shared/testing/fixtures/executive_overview';
import { alertDetail } from './alert-detail';
import * as table from './i18n';

afterEach(() => setLang('th'));

describe('alertDetail', () => {
  it('AD-1 the texts exist in both languages', () => {
    expect(table.th.alertStaleMinutes).toBe('ค้างเกิน {n} นาที');
    expect(table.en.alertStaleMinutes).toBe('Unfinished for over {n} min');
    expect(table.th.alertStock).toBe('{currency} ใช้ได้ {available} · ขั้นต่ำ {floor} (คงเหลือ {onHand} · จองไว้ {reserved})');
    expect(table.en.alertStock).toBe(
      '{currency} available {available} · floor {floor} (on hand {onHand} · reserved {reserved})',
    );
  });

  it('AD-2 ratePending, and a countDifference with no figures, say nothing more', () => {
    expect(alertDetail(makeAlertDto({ kind: 'ratePending' }))).toBe('');
    expect(alertDetail(makeAlertDto({ kind: 'countDifference', differences: [] }))).toBe('');
  });

  it('AD-3 staleData words its minutes', () => {
    const a = makeAlertDto({ kind: 'staleData', staleMinutes: 45 });
    expect(alertDetail(a)).toBe('ค้างเกิน 45 นาที');
    setLang('en');
    expect(alertDetail(a)).toBe('Unfinished for over 45 min');
  });

  it('AD-4 countDifference lists each currency and its amount, in the order sent', () => {
    const a = makeAlertDto({
      kind: 'countDifference',
      differences: [
        { currency: 'THB', amount: '12.50' },
        { currency: 'USD', amount: '-3.00' },
      ],
    });
    expect(alertDetail(a)).toBe('THB 12.50 · USD -3.00');
  });

  it('AD-5 lowStock words its currency and four quantities', () => {
    const a = makeAlertDto({
      kind: 'lowStock',
      stock: { currency: 'USD', available: '40.0000', onHand: '80.0000', reserved: '40.0000', floor: '50.0000' },
    });
    expect(alertDetail(a)).toBe('USD ใช้ได้ 40.0000 · ขั้นต่ำ 50.0000 (คงเหลือ 80.0000 · จองไว้ 40.0000)');
    setLang('en');
    expect(alertDetail(a)).toBe('USD available 40.0000 · floor 50.0000 (on hand 80.0000 · reserved 40.0000)');
  });

  it('AD-6 a figure field the kind does not use is ignored', () => {
    const a = makeAlertDto({ kind: 'ratePending', staleMinutes: 9, differences: [{ currency: 'THB', amount: '1.00' }] });
    expect(alertDetail(a)).toBe('');
  });
});
