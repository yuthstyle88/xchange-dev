// #2508: the "entered at" time on Admin → Starting prices is shown in Bangkok time
// whatever the device's timezone is. Each test sets the device zone through
// process.env.TZ (Node re-reads it on change) and restores it afterwards.

import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { OrgStartingPriceItem } from '../../../shared/api/generated/org_starting_prices';
import { setLang } from '../../../shared/i18n';
import type { AdminMenuProps } from '../menus';
import { StartingPrices } from './StartingPrices';

const ENTERED_AT = '2026-10-15T18:30:00Z';
// 2026-10-16 01:30 in Bangkok, th-TH (Buddhist year 2569).
const BANGKOK_TEXT = '16/10/2569 01:30:00';

// The web tsconfig has no Node types; reach process.env through globalThis.
const env = (globalThis as unknown as { process: { env: Record<string, string | undefined> } }).process.env;

function dummyProps(): AdminMenuProps {
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-10-13', to: '2026-10-13' });
  const [shown] = createSignal(true);
  return { branch, period, shown };
}

describe('#2508: StartingPrices formats the entered-at time in Asia/Bangkok', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | null = null;
  let savedTz: string | undefined;
  const viewItems: OrgStartingPriceItem[] = [
    { currency: 'USD', version: 2, buy: '34.10', sell: '34.70', enteredBy: 'alice', enteredAt: ENTERED_AT, reason: 'v2' },
    { currency: 'USD', version: 1, buy: '34.00', sell: '34.50', enteredBy: 'bob', enteredAt: ENTERED_AT, reason: 'v1' },
  ];

  beforeEach(() => {
    savedTz = env.TZ;
    setLang('th');
    container = document.createElement('div');
    document.body.appendChild(container);
    vi.stubGlobal('fetch', async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(url, 'http://localhost');
      if (u.pathname === '/api/v3/views/org_starting_prices') {
        return new Response(JSON.stringify({ version: 1, changed: true, data: viewItems }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }
      return new Response(JSON.stringify({ error: 'not found' }), { status: 404 });
    });
  });

  afterEach(() => {
    dispose?.();
    dispose = null;
    container.remove();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    if (savedTz === undefined) delete env.TZ;
    else env.TZ = savedTz;
  });

  async function renderedTimes(): Promise<{ current: string; history: string }> {
    dispose = render(() => <StartingPrices {...dummyProps()} />, container);
    await vi.waitFor(() => {
      expect(container.querySelector('[data-currency="USD"] .xc-sp-history-meta')).not.toBeNull();
    });
    const row = container.querySelector('[data-currency="USD"]')!;
    (row.querySelector('.xc-sp-history-toggle') as HTMLButtonElement).click();
    await vi.waitFor(() => {
      expect(row.querySelector('.xc-sp-history-item')).not.toBeNull();
    });
    return {
      current: row.querySelector('.xc-sp-history-meta')!.textContent ?? '',
      history: row.querySelector('.xc-sp-history-item')!.textContent ?? '',
    };
  }

  it('device TZ UTC: 2026-10-15T18:30:00Z is shown as 16 Oct 01:30 (Bangkok, th-TH)', async () => {
    env.TZ = 'UTC';
    // Guard: the device zone really is UTC in this test.
    expect(new Date(ENTERED_AT).getHours()).toBe(18);

    const { current, history } = await renderedTimes();
    expect(current).toContain(BANGKOK_TEXT);
    expect(history).toContain(BANGKOK_TEXT);
    expect(current).not.toContain('15/10/2569');
  });

  it('device TZ Asia/Bangkok: the same text is shown as on a UTC device', async () => {
    env.TZ = 'Asia/Bangkok';
    expect(new Date(ENTERED_AT).getHours()).toBe(1);
    const onBangkok = await renderedTimes();
    expect(onBangkok.current).toContain(BANGKOK_TEXT);
    expect(onBangkok.history).toContain(BANGKOK_TEXT);

    // Same data on a UTC device must give the exact same text.
    dispose?.();
    dispose = null;
    container.textContent = '';
    env.TZ = 'UTC';
    expect(new Date(ENTERED_AT).getHours()).toBe(18);
    const onUtc = await renderedTimes();
    expect(onUtc).toEqual(onBangkok);
  });
});
