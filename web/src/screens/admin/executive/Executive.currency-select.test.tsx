import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { makeExecutiveOverviewData } from '../../../shared/testing/fixtures';
import { setLang } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';

const loaded = import.meta.glob<{ menu: AdminMenu }>('./menu.ts', { eager: true });
function theMenu(): AdminMenu {
  const m = loaded['./menu.ts'];
  expect(m).toBeDefined();
  return m!.menu;
}

const ENDPOINT = '/api/v3/views/executive_overview';
const VERSIONS = { B1: 1 };

interface Sent {
  path: string;
  params: URLSearchParams;
}

let sent: Sent[] = [];
function fakeFetch(input: unknown): Promise<Response> {
  const url = new URL(String(input), 'http://desk.test');
  sent.push({ path: url.pathname, params: url.searchParams });
  const data = makeExecutiveOverviewData({
    period: { from: '2026-10-01', to: '2026-10-08' },
    kpis: [],
    branches: [],
    alerts: [],
    definitions: [],
  });
  return Promise.resolve(
    new Response(
      JSON.stringify({ version: 1, versions: VERSIONS, changed: true, data }),
      {
        status: 200,
        headers: { 'content-type': 'application/json' },
      },
    ),
  );
}

function reads(): Sent[] {
  return sent.filter((s) => s.path === ENDPOINT);
}

function last(): Sent {
  const all = reads();
  expect(all.length).toBeGreaterThan(0);
  return all[all.length - 1]!;
}

const realSetTimeout = globalThis.setTimeout;
async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) {
    if (globalThis.setTimeout !== realSetTimeout) await vi.advanceTimersByTimeAsync(0);
    else await new Promise((r) => realSetTimeout(r, 0));
  }
}

function change(el: HTMLInputElement): void {
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
}

let dispose: (() => void) | undefined;
let root: HTMLElement;

function mount(): HTMLElement {
  const menu = theMenu();
  const [branch] = createSignal('B1');
  const [period] = createSignal({ from: '2026-10-01', to: '2026-10-08' });
  const [shown] = createSignal(true);
  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(
    () => <menu.Component branch={branch} period={period} shown={shown} />,
    root,
  );
  return root;
}

beforeEach(() => {
  sent = [];
  vi.stubGlobal('fetch', vi.fn(fakeFetch));
  location.hash = '#executive';
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

describe('F3: Executive currency text input validation', () => {
  it('F3: typing us1 in currency shows worded line at the field and no read with that currency; typing usd reads with USD', async () => {
    const el = mount();
    await settle();
    const initialReadsCount = reads().length;
    expect(initialReadsCount).toBe(1);

    const input = el.querySelector<HTMLInputElement>('input[name="currency"]');
    expect(input, 'input[name="currency"] must exist').not.toBeNull();

    // Type "us1"
    input!.value = 'us1';
    change(input!);
    await settle();

    // No new read with "US1" or "us1"
    expect(reads().length).toBe(initialReadsCount);
    expect(reads().some((r) => r.params.get('currency')?.toUpperCase() === 'US1')).toBe(false);

    // Worded line at the field exists
    const errorEl = el.querySelector('[data-field-error="currency"], .field-error');
    expect(errorEl, 'worded line at the currency field must exist').not.toBeNull();
    expect(errorEl?.textContent?.trim().length).toBeGreaterThan(0);

    // Now type "usd"
    input!.value = 'usd';
    change(input!);
    await settle();

    // Worded line cleared
    const clearedError = el.querySelector('[data-field-error="currency"], .field-error');
    expect(clearedError).toBeNull();

    // Now reads with USD
    expect(reads().length).toBe(initialReadsCount + 1);
    expect(last().params.get('currency')).toBe('USD');
  });
});
