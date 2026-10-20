// A03 rate publication history extra tests (#3438, #564; BE #3399).
// Covers the four "new" rows of the behaviour table in #3438 §9.
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from '../../../shared/api/client';
import { setLang } from '../../../shared/i18n';
import * as table from './i18n';
import {
  makeRatePublicationRow,
  type RatePublicationLine,
  type RatePublicationRow,
} from '../../../shared/testing/fixtures';
import { RatesHistory } from './RatesHistory';

function say(key: string): string {
  return (table.th as Record<string, string>)[key]!;
}

function row(version: number, by: string, state: string, lines?: RatePublicationLine[]) {
  return makeRatePublicationRow({
    id: `pub-${version}`,
    branchId: 'MAIN',
    version,
    publishedAt: `2026-10-${String(10 + (version % 9)).padStart(2, '0')}T03:00:00Z`,
    publishedBy: by,
    approvedRequestId: `req-${version}`,
    draftId: `d-${version}`,
    state,
    lines: lines ?? [],
  });
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
async function mount(first?: RatePublicationRow[]): Promise<HTMLDivElement> {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <RatesHistory client={client} branch="MAIN" first={first} />, host);
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

describe('A03 history extra behaviour rows (#3438 §9)', () => {
  it('mounted with first (2 rows): no read on mount and those 2 rows are shown', async () => {
    await mount([row(5, 'owner-1', 'PUBLISHED'), row(4, 'rates-2', 'SUPERSEDED')]);
    expect(calls).toHaveLength(0);
    const shown = [...host.querySelectorAll('[data-publication]')].map((e) => e.getAttribute('data-publication'));
    expect(shown).toEqual(['5', '4']);
    expect(more()).toBeUndefined();
  });

  it('mounted with first of 20 rows, then "more": reads before = the 20th row id', async () => {
    const first = Array.from({ length: 20 }, (_, i) => row(30 - i, 'owner-1', i === 0 ? 'PUBLISHED' : 'SUPERSEDED'));
    answer = async () => [row(10, 'owner-1', 'SUPERSEDED')];
    await mount(first);
    expect(calls).toHaveLength(0);
    expect(more()).toBeDefined();

    more()!.click();
    await settle();
    expect(calls).toEqual([{ name: 'rate_publications', q: { branch: 'MAIN', limit: 20, before: 'pub-11' } }]);
    expect(host.querySelectorAll('[data-publication]')).toHaveLength(21);
    expect(pub(10)).not.toBeNull();
  });

  it('"More" fails: the rows already shown stay, the error is shown, and "more" stays to retry', async () => {
    const first = Array.from({ length: 20 }, (_, i) => row(20 - i, 'owner-1', i === 0 ? 'PUBLISHED' : 'SUPERSEDED'));
    answer = async () => {
      throw new ApiError({ code: 'serverError', message: 'โหลดเพิ่มไม่สำเร็จ', status: 500 });
    };
    await mount(first);
    expect(host.querySelectorAll('[data-publication]')).toHaveLength(20);

    more()!.click();
    await settle();
    expect(host.querySelectorAll('[data-publication]')).toHaveLength(20);
    const err = host.querySelector('[data-history-error]');
    expect(err).not.toBeNull();
    expect(text(err)).toContain('เซิร์ฟเวอร์ขัดข้อง');
    expect(more()).toBeDefined();
  });

  it('a row with lines: [] shows the row with no line table', async () => {
    await mount([row(9, 'owner-1', 'PUBLISHED', [])]);
    const el = pub(9);
    expect(el).not.toBeNull();
    expect(text(el)).toContain('owner-1');
    expect(el!.querySelector('table')).toBeNull();
    expect(el!.querySelector('[data-history-line]')).toBeNull();
  });
});
