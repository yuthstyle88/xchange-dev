// F56b (docs/design/backdated-keying.md §16.3): the backdated form shows the SAFE balance it posts against, read from
// `views/backdate_safe`, and links to /staff where a top-up is keyed. Written before the implementation by the ticket
// author claude/3b81c9d5. Do not edit, delete or weaken; add tests in a file of your own.
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiClient } from '../../shared/api/client';
import { setLang } from '../../shared/i18n';
import { SafeBalance } from './SafeBalance';

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let reads: URL[];
let balances: Array<{ currency: string; amount: string }>;

function api(): ApiClient {
  return new ApiClient(async (url) => {
    const u = new URL(url, 'http://desk.test');
    if (u.pathname === '/api/v3/views/backdate_safe') {
      reads.push(u);
      return new Response(JSON.stringify({ version: reads.length, changed: true, data: { balances } }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });
    }
    return new Response(JSON.stringify({ version: 1, changed: false }), { status: 200 });
  });
}

async function settle(): Promise<void> {
  for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 10));
}

beforeEach(() => {
  setLang('th');
  reads = [];
  balances = [
    { currency: 'THB', amount: '150000.00' },
    { currency: 'USD', amount: '520.00' },
  ];
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
});

describe('F56b: the SAFE balance beside the backdated form', () => {
  it('reads backdate_safe for the branch and shows each currency, grouped', async () => {
    const [refresh] = createSignal(0);
    const client = api();
    dispose = render(() => <SafeBalance api={client} branch="MAIN" refresh={refresh} />, host);
    await settle();
    expect(reads).toHaveLength(1);
    expect(reads[0]!.searchParams.get('branch')).toBe('MAIN');
    const box = host.querySelector('[data-testid="safe-balance"]');
    expect(box, 'the balance list').not.toBeNull();
    const text = (box!.textContent ?? '').replace(/\s+/g, ' ');
    expect(text).toContain('THB 150,000.00');
    expect(text).toContain('USD 520.00');
    expect(text.indexOf('THB')).toBeLessThan(text.indexOf('USD'));
  });

  it('an empty SAFE says so instead of showing nothing', async () => {
    balances = [];
    const [refresh] = createSignal(0);
    const client = api();
    dispose = render(() => <SafeBalance api={client} branch="MAIN" refresh={refresh} />, host);
    await settle();
    expect(host.querySelector('[data-testid="safe-empty"]')).not.toBeNull();
  });

  it('links to /staff, where a top-up is keyed', async () => {
    const [refresh] = createSignal(0);
    const client = api();
    dispose = render(() => <SafeBalance api={client} branch="MAIN" refresh={refresh} />, host);
    await settle();
    expect(host.querySelector<HTMLAnchorElement>('[data-testid="safe-transfer-link"]')?.getAttribute('href')).toBe('/staff');
  });

  it('reads again when the form bumps refresh after a save', async () => {
    const [refresh, setRefresh] = createSignal(0);
    const client = api();
    dispose = render(() => <SafeBalance api={client} branch="MAIN" refresh={refresh} />, host);
    await settle();
    expect(reads).toHaveLength(1);
    balances = [{ currency: 'THB', amount: '133925.00' }];
    setRefresh(1);
    await settle();
    expect(reads).toHaveLength(2);
    expect(host.querySelector('[data-testid="safe-balance"]')?.textContent ?? '').toContain('THB 133,925.00');
  });

  it('reads nothing until the branch is known', async () => {
    const [refresh] = createSignal(0);
    const client = api();
    dispose = render(() => <SafeBalance api={client} branch="" refresh={refresh} />, host);
    await settle();
    expect(reads).toHaveLength(0);
  });
});
