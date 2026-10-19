// A03 rate publication history (survey xchange#564 issuecomment-6262359159 row A03; BE #3399): the history
// pages through the branch's publications, newest first, and shows each version's lines before → after.
// Written before the implementation by the sub-controller (claude/dell-claude-3). Do not edit, delete or
// weaken; add tests in a file of your own.
//
// Design: `docs/design/blueprint/rates.md:451` ("paged view (A03) … versions, who, when, before/after per
// line"). The wire is #3399 §7-8: `GET /api/v3/views/rate_publications?branch=&limit=&before=<publication id>`,
// `data` = rows newest first (`RatePublicationRow`, local type in `./rate-history.ts` until the generated one lands).
//
// The API this test pins ([required]):
// - `RatesHistory({ client, branch })` (`./RatesHistory.tsx`) reads `client.page('rate_publications',
//   { branch, limit: 20 })`, then `{ branch, limit: 20, before: <id of the last row shown> }` for "more";
// - `lineChange(line)` (`./rate-history.ts`): `added` (before null), `dropped` (after null), `changed`
//   (buy or sell differs), `same`.
// DOM hooks: one `[data-publication="<version>"]` per row, holding the version, `publishedBy` and the state
// word (`statePublished` / `stateSuperseded`); in it, one `[data-history-line="<currency>|<group>|<valueTier>"]`
// per line with `data-line-change="<lineChange>"`, the before and after buy/sell; a button whose text is
// `historyMore` while the last page was full (20 rows), absent after a short page; a failed read shows its
// worded message in `[data-history-error]`.
// Keys of `./i18n.ts` (th and en): `historyMore`, `statePublished`, `stateSuperseded`, `headerBefore`,
// `headerAfter`, `lineAdded`, `lineDropped`.
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from '../../../shared/api/client';
import { setLang } from '../../../shared/i18n';
import * as table from './i18n';
import * as history from './rate-history';
import type { RatePublicationLine, RatePublicationRow } from './rate-history';
import { RatesHistory } from './RatesHistory';

const THAI = /[฀-๿]/;
type Lang = 'th' | 'en';
function say(key: string, lang: Lang = 'th'): string {
  const v = (table as unknown as Record<Lang, Record<string, string>>)[lang]?.[key];
  expect(v, `admin.rates i18n ${lang}.${key}`).toBeTruthy();
  return v!;
}

const sides = (buy: string | null, sell: string | null) => ({ buy, sell });
function line(currency: string, before: RatePublicationLine['before'], after: RatePublicationLine['after'], tier = 'ALL'): RatePublicationLine {
  return { currency, group: 'STD', valueTier: tier, unitSize: currency === 'JPY' ? '100' : '1', before, after };
}
function row(version: number, by: string, state: string, lines: RatePublicationLine[] = []): RatePublicationRow {
  return {
    id: `pub-${version}`,
    branchId: 'MAIN',
    version,
    publishedAt: `2026-10-${String(10 + (version % 9)).padStart(2, '0')}T03:00:00Z`,
    publishedBy: by,
    approvedRequestId: null,
    draftId: `d-${version}`,
    state,
    lines,
  };
}

let calls: Array<{ name: string; q: Record<string, unknown> }>;
let answer: (q: Record<string, unknown>) => Promise<RatePublicationRow[]>;
const client = {
  page: vi.fn(async (name: string, q: Record<string, unknown> = {}) => {
    calls.push({ name, q });
    return answer(q);
  }),
} as unknown as Parameters<typeof RatesHistory>[0]['client'];

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) await new Promise((r) => setTimeout(r, 0));
}
async function mount(): Promise<HTMLDivElement> {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <RatesHistory client={client} branch="MAIN" />, host);
  await settle();
  return host;
}
const text = (el: Element | null | undefined) => (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
const pub = (v: number) => host.querySelector(`[data-publication="${v}"]`);
const more = () => [...host.querySelectorAll('button')].find((b) => text(b) === say('historyMore'));

beforeEach(() => {
  calls = [];
  setLang('th');
});
afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
});

describe('A03 history: lineChange (rate-history.ts)', () => {
  it('names added, dropped, changed and same lines', () => {
    expect(history.lineChange(line('EUR', null, sides('40.00000000', '41.00000000')))).toBe('added');
    expect(history.lineChange(line('JPY', sides('0.25000000', '0.26000000'), null))).toBe('dropped');
    expect(history.lineChange(line('USD', sides('35.00000000', '36.00000000'), sides('35.10000000', '36.00000000')))).toBe('changed');
    expect(history.lineChange(line('USD', sides('35.00000000', '36.00000000'), sides('35.00000000', '36.00000000')))).toBe('same');
  });
});

describe('A03 history: the paged read (rates.md:451, #3399 §8)', () => {
  it('reads rate_publications for the branch with limit 20 and lists the rows newest first', async () => {
    answer = async () => [row(3, 'rates-9', 'PUBLISHED'), row(2, 'owner-1', 'SUPERSEDED'), row(1, 'owner-1', 'SUPERSEDED')];
    await mount();
    expect(calls[0]).toEqual({ name: 'rate_publications', q: { branch: 'MAIN', limit: 20 } });
    const shown = [...host.querySelectorAll('[data-publication]')].map((e) => e.getAttribute('data-publication'));
    expect(shown).toEqual(['3', '2', '1']);
    expect(text(pub(3))).toContain('rates-9');
    expect(text(pub(3))).toContain(say('statePublished'));
    expect(text(pub(2))).toContain(say('stateSuperseded'));
  });

  it('a full page shows "more", which reads before the last row id and appends; a short page hides it', async () => {
    const first = Array.from({ length: 20 }, (_, i) => row(40 - i, 'owner-1', i === 0 ? 'PUBLISHED' : 'SUPERSEDED'));
    answer = async (q) => (q.before === undefined ? first : [row(20, 'owner-1', 'SUPERSEDED')]);
    await mount();
    expect(more(), 'a full page offers more').toBeDefined();
    more()!.click();
    await settle();
    expect(calls[1]).toEqual({ name: 'rate_publications', q: { branch: 'MAIN', limit: 20, before: 'pub-21' } });
    expect(host.querySelectorAll('[data-publication]').length).toBe(21);
    expect(pub(20)).not.toBeNull();
    expect(more(), 'a short page ends the list').toBeUndefined();
  });

  it('a short first page offers no "more"', async () => {
    answer = async () => [row(1, 'owner-1', 'PUBLISHED')];
    await mount();
    expect(more()).toBeUndefined();
  });

  it('a failed read shows its worded message, not nothing', async () => {
    answer = async () => {
      throw new ApiError({ code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง', status: 500 });
    };
    await mount();
    const err = host.querySelector('[data-history-error]');
    expect(err, 'the error is shown').not.toBeNull();
    expect(text(err)).not.toBe('');
  });
});

describe('A03 history: what each version changed (#3399 §7)', () => {
  const lines = [
    line('USD', sides('35.00000000', '36.00000000'), sides('35.10000000', '36.00000000')),
    line('EUR', null, sides('40.00000000', '41.00000000')),
    line('JPY', sides('0.25000000', '0.26000000'), null),
    line('GBP', sides('44.00000000', '45.00000000'), sides('44.00000000', '45.00000000')),
  ];

  it('each line carries its change and shows before and after', async () => {
    answer = async () => [row(7, 'rates-9', 'PUBLISHED', lines)];
    await mount();
    const l = (c: string) => pub(7)!.querySelector(`[data-history-line="${c}|STD|ALL"]`);
    expect(l('USD')?.getAttribute('data-line-change')).toBe('changed');
    expect(l('EUR')?.getAttribute('data-line-change')).toBe('added');
    expect(l('JPY')?.getAttribute('data-line-change')).toBe('dropped');
    expect(l('GBP')?.getAttribute('data-line-change')).toBe('same');
    expect(text(l('USD'))).toContain('35');
    expect(text(l('USD'))).toContain('35.1');
    expect(text(l('EUR'))).toContain(say('lineAdded'));
    expect(text(l('JPY'))).toContain(say('lineDropped'));
  });

  it('two value tiers of one currency are two lines', async () => {
    answer = async () => [
      row(8, 'rates-9', 'PUBLISHED', [
        line('USD', sides('35.00000000', '36.00000000'), sides('35.10000000', '36.00000000'), 'T1'),
        line('USD', sides('34.90000000', '36.10000000'), sides('34.90000000', '36.10000000'), 'T2'),
      ]),
    ];
    await mount();
    expect(pub(8)!.querySelector('[data-history-line="USD|STD|T1"]')?.getAttribute('data-line-change')).toBe('changed');
    expect(pub(8)!.querySelector('[data-history-line="USD|STD|T2"]')?.getAttribute('data-line-change')).toBe('same');
  });
});

describe('A03 history: Thai and English (W10)', () => {
  it('every key this test reads is in both tables, the English one with no Thai', () => {
    for (const k of ['historyMore', 'statePublished', 'stateSuperseded', 'headerBefore', 'headerAfter', 'lineAdded', 'lineDropped']) {
      say(k, 'th');
      expect(THAI.test(say(k, 'en')), `en.${k} has no Thai`).toBe(false);
    }
  });
});
