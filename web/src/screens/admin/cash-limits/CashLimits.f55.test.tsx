// F55 (docs/design/backdated-keying.md §16.7), restated by the ticket author claude/3b81c9d5: dates are dd/mm/yyyy HH:mm
// (CE, Bangkok) from the shared formatter.
// The banner's "set by … · {updatedAt}" is an instant, so it reads dd/mm/yyyy HH:mm.
// Written before the implementation by the ticket author. Do not edit, delete or weaken; add tests in a file of your own.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { makeCashLimitsData, makeMe } from '../../../shared/testing/fixtures';
import { setLang } from '../../../shared/i18n';
import { sharedSession } from '../../../shared/session/session';
import { CashLimits } from './CashLimits';

const LIMIT_AT = '2026-11-02T17:30:00Z'; // 03/11/2026 00:30 in Bangkok (the UTC day is 02/11)

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

let dispose: (() => void) | undefined;
let root: HTMLElement;
const text = (el: Element | null | undefined) => (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
const banner = () => root.querySelector<HTMLElement>('[data-current-limit]');

async function mount(): Promise<void> {
  const answer = makeCashLimitsData({
    branch: 'MAIN',
    countDifferenceLimitThb: '50.00',
    updatedBy: 'u-finance',
    updatedByName: 'การเงิน ข',
    updatedAt: LIMIT_AT,
  });
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) =>
      String(input).includes('/api/v3/views/cash_limits')
        ? json(200, { version: 1, changed: true, data: answer })
        : json(404, { error: { code: 'notFound', message: 'nf' } }),
    ),
  );
  root = document.createElement('div');
  document.body.appendChild(root);
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-11-03', to: '2026-11-03' });
  dispose = render(() => <CashLimits branch={branch} period={period} shown={() => true} />, root);
  await vi.waitFor(() => expect(text(banner())).toContain('50.00'));
}

beforeEach(() => {
  sharedSession().updateMe(makeMe({ permissions: ['CashLimitsManage'] }));
  setLang('th');
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  root?.remove();
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  setLang('th');
});

describe('F55: the cash-limit banner shows its time as dd/mm/yyyy HH:mm', () => {
  it('F55-CL1 th: the banner ends with 03/11/2026 00:30 (Bangkok, CE, no seconds)', async () => {
    await mount();
    expect(text(banner())).toMatch(/· ตั้งโดย การเงิน ข · 03\/11\/2026 00:30$/);
  });

  it('F55-CL2 en: the same text, never a locale form', async () => {
    setLang('en');
    await mount();
    expect(text(banner())).toMatch(/· set by การเงิน ข · 03\/11\/2026 00:30$/);
  });
});
