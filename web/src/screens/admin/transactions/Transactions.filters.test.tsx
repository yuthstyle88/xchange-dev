import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { makeTransactionRow } from '../../../shared/testing/fixtures';
import { setLang } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';

const loadedTransactions = import.meta.glob<{ menu: AdminMenu }>('./menu.ts', { eager: true });
function transactionsMenu(): AdminMenu {
  const m = loadedTransactions['./menu.ts'];
  expect(m).toBeDefined();
  return m!.menu;
}

const loadedReports = import.meta.glob<{ menu: AdminMenu }>('../reports/menu.ts', { eager: true });
function reportsMenu(): AdminMenu {
  const m = loadedReports['../reports/menu.ts'];
  expect(m).toBeDefined();
  return m!.menu;
}

interface RequestLog {
  path: string;
  view: string;
  params: URLSearchParams;
}

let requests: RequestLog[] = [];
function fakeFetch(input: unknown): Promise<Response> {
  const url = new URL(String(input), 'http://desk.test');
  const view = url.pathname.startsWith('/api/v3/views/') ? url.pathname.slice('/api/v3/views/'.length) : '';
  requests.push({ path: url.pathname, view, params: url.searchParams });

  if (view === 'admin_transactions') {
    const data = [
      makeTransactionRow({ id: 'TX-1', currency: 'USD' }),
    ];
    return Promise.resolve(new Response(JSON.stringify({ version: 1, changed: true, data }), {
      status: 200,
      headers: { 'content-type': 'application/json' },
    }));
  }

  if (view === 'admin_reports') {
    return Promise.resolve(new Response(JSON.stringify({
      version: 1,
      changed: true,
      data: {
        cards: [],
        payments: [],
        currencies: [],
        employeeTotals: { people: 0, completed: 0, received: '0.00', paid: '0.00', net: '0.00' },
        employees: [],
      },
    }), {
      status: 200,
      headers: { 'content-type': 'application/json' },
    }));
  }

  return Promise.resolve(new Response(JSON.stringify({ version: 1, changed: true, data: {} }), {
    status: 200,
    headers: { 'content-type': 'application/json' },
  }));
}

function txReads(): URLSearchParams[] {
  return requests.filter((r) => r.view === 'admin_transactions').map((r) => r.params);
}

const realSetTimeout = globalThis.setTimeout;
async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) {
    if (globalThis.setTimeout !== realSetTimeout) await vi.advanceTimersByTimeAsync(0);
    else await new Promise((r) => realSetTimeout(r, 0));
  }
}

let dispose: (() => void) | undefined;
let root: HTMLElement;

function mountTransactions(): HTMLElement {
  const menu = transactionsMenu();
  const [branch] = createSignal('B1');
  const [period] = createSignal({ from: '2026-10-01', to: '2026-10-08' });
  const [shown] = createSignal(true);
  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(() => <menu.Component branch={branch} period={period} shown={shown} />, root);
  return root;
}

function mountReports(): HTMLElement {
  const menu = reportsMenu();
  const [branch] = createSignal('B1');
  const [period] = createSignal({ from: '2026-10-01', to: '2026-10-08' });
  const [shown] = createSignal(true);
  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(() => <menu.Component branch={branch} period={period} shown={shown} />, root);
  return root;
}

beforeEach(() => {
  requests = [];
  vi.stubGlobal('fetch', vi.fn(fakeFetch));
  setLang('th');
});

afterEach(async () => {
  dispose?.();
  dispose = undefined;
  root?.remove();
  vi.unstubAllGlobals();
  location.hash = '';
  await settle();
});

describe('Transactions and Reports search filters', () => {
  it('F1: Transactions search input has maxLength 100', async () => {
    location.hash = '#transactions';
    const el = mountTransactions();
    await settle();
    const input = el.querySelector<HTMLInputElement>('input[type="search"]');
    expect(input, 'search input exists').not.toBeNull();
    expect(input!.maxLength).toBe(100);
  });

  it('F2: Reports staff search input has maxLength 100', async () => {
    location.hash = '#reports';
    const el = mountReports();
    await settle();
    const input = el.querySelector<HTMLInputElement>('#employee-search');
    expect(input, 'staff search input exists').not.toBeNull();
    expect(input!.maxLength).toBe(100);
  });

  it('F4: Transactions opens #admin/transactions?currency=USD&channel=CASH and reads with those filters, showing active badges with clear buttons', async () => {
    location.hash = '#admin/transactions?currency=USD&channel=CASH';
    const el = mountTransactions();
    await settle();

    // Verify initial read carries currency=USD and channel=CASH
    const initialReads = txReads();
    expect(initialReads.length).toBeGreaterThan(0);
    const firstParams = initialReads[0]!;
    expect(firstParams.get('currency')).toBe('USD');
    expect(firstParams.get('channel')).toBe('CASH');

    // Both show as active filter badges with a clear button
    const currencyBadge = el.querySelector('[data-filter="currency"]');
    expect(currencyBadge, 'currency badge must be visible').not.toBeNull();
    expect(currencyBadge!.textContent).toContain('USD');
    const clearCurrencyBtn = currencyBadge!.querySelector('button');
    expect(clearCurrencyBtn, 'clear button on currency badge must exist').not.toBeNull();

    const channelBadge = el.querySelector('[data-filter="channel"]');
    expect(channelBadge, 'channel badge must be visible').not.toBeNull();
    expect(channelBadge!.textContent).toContain('CASH');
    const clearChannelBtn = channelBadge!.querySelector('button');
    expect(clearChannelBtn, 'clear button on channel badge must exist').not.toBeNull();

    // Clear currency
    clearCurrencyBtn!.click();
    await settle();

    expect(el.querySelector('[data-filter="currency"]')).toBeNull();
    const lastParams = txReads()[txReads().length - 1]!;
    expect(lastParams.has('currency')).toBe(false);
    expect(lastParams.get('channel')).toBe('CASH');
  });
});
