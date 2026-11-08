// BK-CP (round 13, parent #564): before a branch is picked, /app/book's currency <select> showed the BRANCH prompt
// "เลือกสาขา…" (`t('selectBranch')`, Book.tsx:759) as its only option, so the currency field read like a second
// branch picker. Written before the implementation by claude-2 for the one-look sub-controller claude/1cc51615.
// Do not edit, delete or weaken these tests; add your own elsewhere.
//
// [required] by these tests:
// - web/src/screens/customer/book/i18n.ts gains one key in th, en and zh:
//     currencyNeedsBranch: 'เลือกสาขาก่อน' | 'Select a branch first' | '请先选择分行'
// - With no branch chosen, the currency select's only option is value "", text t('currencyNeedsBranch'), disabled.
// - Once a branch is chosen the placeholder is gone and the branch's currencies are listed (FE-T2c unchanged).
// - The branch select's own "เลือกสาขา…" placeholder is unchanged.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { makePublishedLineDto, makePublishedRatesData } from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { warmModules } from '../../../shared/testing/warm-modules';
import * as bookI18n from './i18n';
import { BookScreen } from './Book';

const BRANCH_ID = 'XC-B01';
const BRANCHES = {
  branches: [{ id: BRANCH_ID, code: BRANCH_ID, name: 'สาขาสยามพารากอน', timezone: 'Asia/Bangkok', bookingEnabled: true }],
};
const SLOTS = { version: 1, changed: true, data: { slots: [] } };

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

async function flush(): Promise<void> {
  for (let i = 0; i < 15; i++) await new Promise((r) => setTimeout(r, 0));
}

/** Read a key through a string index, so the file compiles before the key exists (it then reads undefined). */
function key(table: Record<string, string>, k: string): string | undefined {
  return table[k];
}

describe('Customer Book: the currency placeholder asks for a branch first (BK-CP)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let fetchStub: (input: string, init?: RequestInit) => Promise<Response>;

  beforeAll(() => warmModules(() => import('./Book')), 60_000);

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    window.history.replaceState({}, '', '/app/book');
    document.body.innerHTML = '';
    host = document.createElement('div');
    document.body.appendChild(host);
    fetchStub = vi.fn(async (input: string): Promise<Response> => {
      const u = new URL(input, 'http://xc.test');
      if (u.pathname === '/api/public/host') return json(200, { org: 'XC', env: 'test' });
      if (u.pathname === `/api/public/orgs/XC/branches/${BRANCH_ID}/rates`) {
        return json(
          200,
          makePublishedRatesData({
            lines: [makePublishedLineDto({ currency: 'USD', buyRate: '35.10', sellRate: '35.40', lineId: 'line-USD' })],
          }),
        );
      }
      if (u.pathname.startsWith('/api/public/orgs/')) return json(200, BRANCHES);
      if (u.pathname === '/api/v3/views/booking_slots') return json(200, SLOTS);
      return json(404, { error: { code: 'notFound', message: u.pathname } });
    });
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    document.body.innerHTML = '';
    window.history.replaceState({}, '', '/app/book');
    vi.restoreAllMocks();
    setAudience('customer');
    setLang('th');
  });

  const currencySelect = () => host.querySelector<HTMLSelectElement>('select[name="currency"]')!;
  const branchSelect = () => host.querySelector<HTMLSelectElement>('select[name="branchId"]')!;

  async function mount(): Promise<void> {
    dispose = render(() => <BookScreen fetch={fetchStub} ratesIntervalMs={60_000} />, host);
    await flush();
    await vi.waitFor(() => expect(host.querySelector(`option[value="${BRANCH_ID}"]`)).not.toBeNull());
  }

  it('BK-CP-1 i18n has currencyNeedsBranch in th, en and zh with the exact texts', () => {
    expect(key(bookI18n.th, 'currencyNeedsBranch')).toBe('เลือกสาขาก่อน');
    expect(key(bookI18n.en, 'currencyNeedsBranch')).toBe('Select a branch first');
    expect(key(bookI18n.zh, 'currencyNeedsBranch')).toBe('请先选择分行');
  });

  it('BK-CP-2 no branch (th): the only currency option is "", disabled, reading เลือกสาขาก่อน (not เลือกสาขา…)', async () => {
    await mount();
    const opts = Array.from(currencySelect().options);
    expect(opts.map((o) => o.value)).toEqual(['']);
    expect(opts[0]!.textContent?.trim()).toBe('เลือกสาขาก่อน');
    expect(opts[0]!.disabled).toBe(true);
    expect(currencySelect().textContent).not.toContain('เลือกสาขา…');
  });

  it('BK-CP-3 no branch (en): the currency placeholder reads Select a branch first', async () => {
    setLang('en');
    await mount();
    const opts = Array.from(currencySelect().options);
    expect(opts.map((o) => o.value)).toEqual(['']);
    expect(opts[0]!.textContent?.trim()).toBe('Select a branch first');
    expect(opts[0]!.disabled).toBe(true);
  });

  it('BK-CP-4 the branch select keeps its own เลือกสาขา… placeholder', async () => {
    await mount();
    const first = branchSelect().options[0]!;
    expect(first.value).toBe('');
    expect(first.textContent?.trim()).toBe('เลือกสาขา…');
  });

  it('BK-CP-5 after a branch is chosen the placeholder is gone and USD is offered', async () => {
    await mount();
    branchSelect().value = BRANCH_ID;
    branchSelect().dispatchEvent(new Event('change', { bubbles: true }));
    await flush();
    await vi.waitFor(() =>
      expect(Array.from(currencySelect().options).map((o) => o.value)).toEqual(['USD']),
    );
    expect(currencySelect().textContent).not.toContain('เลือกสาขาก่อน');
  });
});
