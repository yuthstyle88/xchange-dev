// B03 tier badge tests (FE-desk, #564, #3343). Tests every row of the behaviour table.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang, t } from '../../../../shared/i18n';
import { ApiClient } from '../../../../shared/api/client';
import { TierBadge } from './TierBadge';
import { th } from './i18n';

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  setLang('th');
});

function mount(el: () => JSX.Element) {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(el, host);
}

const sleep = (ms = 20) => new Promise((resolve) => setTimeout(resolve, ms));

describe('TierBadge component', () => {
  it('row 1: tier = "GOLD" shows the tier code with the tier label', async () => {
    const api = {
      view: vi.fn(async () => ({
        version: 1,
        changed: false,
        data: { tier: 'GOLD' },
      })),
    } as unknown as ApiClient;

    mount(() => (
      <TierBadge api={api} branch="MAIN" customerId="c-1" refreshKey={0} />
    ));

    await sleep();

    expect(host.textContent).toContain(th.tier);
    expect(host.textContent).toContain('GOLD');
    expect(api.view).toHaveBeenCalledWith(
      'customer_tier',
      expect.objectContaining({
        branch: 'MAIN',
        customer: 'c-1',
      }),
    );
  });

  it('row 2: tier = null shows a "no tier yet" text without error', async () => {
    const api = {
      view: vi.fn(async () => ({
        version: 1,
        changed: false,
        data: { tier: null },
      })),
    } as unknown as ApiClient;

    mount(() => (
      <TierBadge api={api} branch="MAIN" customerId="c-1" refreshKey={0} />
    ));

    await sleep();

    expect(host.textContent).toContain(th.noTier);
    expect(host.querySelector('[role="alert"]')).toBeNull();
    expect(host.querySelector('.desk-error')).toBeNull();
  });

  it('row 3: read refused / offline shows a short worded note and no alert', async () => {
    const api = {
      view: vi.fn(async () => {
        throw new Error('connection offline');
      }),
    } as unknown as ApiClient;

    mount(() => (
      <TierBadge api={api} branch="MAIN" customerId="c-1" refreshKey={0} />
    ));

    await sleep();

    expect(host.textContent).toContain(t('error.network'));
    expect(host.textContent).not.toContain('connection offline');
    expect(host.querySelector('[role="alert"]')).toBeNull();
    expect(host.querySelector('.desk-notice')).not.toBeNull();
  });

  it('row 4: refreshKey changes triggers one more read', async () => {
    const [refreshKey, setRefreshKey] = createSignal(0);
    const api = {
      view: vi.fn(async () => ({
        version: 1,
        changed: false,
        data: { tier: 'SILVER' },
      })),
    } as unknown as ApiClient;

    mount(() => (
      <TierBadge
        api={api}
        branch="MAIN"
        customerId="c-1"
        refreshKey={refreshKey()}
      />
    ));

    await sleep();
    expect(api.view).toHaveBeenCalledTimes(1);

    setRefreshKey(1);
    await sleep();
    expect(api.view).toHaveBeenCalledTimes(2);
  });

  it('row 5: customerId changes before first answer shows only the newest answer', async () => {
    const [customerId, setCustomerId] = createSignal('c-1');

    let resolveFirst: ((val: any) => void) | undefined;
    const firstPromise = new Promise((resolve) => {
      resolveFirst = resolve;
    });

    const api = {
      view: vi.fn(async (_name: string, q: any) => {
        if (q.customer === 'c-1') {
          return firstPromise;
        }
        return {
          version: 1,
          changed: false,
          data: { tier: 'PLATINUM' },
        };
      }),
    } as unknown as ApiClient;

    mount(() => (
      <TierBadge
        api={api}
        branch="MAIN"
        customerId={customerId()}
        refreshKey={0}
      />
    ));

    // First request started
    expect(api.view).toHaveBeenCalledTimes(1);

    // Switch customer to c-2 before first completes
    setCustomerId('c-2');
    await sleep();
    expect(api.view).toHaveBeenCalledTimes(2);
    expect(host.textContent).toContain('PLATINUM');

    // Now resolve first promise with stale data
    resolveFirst?.({
      version: 1,
      changed: false,
      data: { tier: 'OLD_BRONZE' },
    });
    await sleep();

    // Still PLATINUM, stale answer dropped
    expect(host.textContent).toContain('PLATINUM');
    expect(host.textContent).not.toContain('OLD_BRONZE');
  });

  it('row 6: a roleNotAllowed refusal shows the worded roleNotAllowed text, not raw server text', async () => {
    const api = new ApiClient(async () =>
      new Response(
        JSON.stringify({
          error: { code: 'roleNotAllowed', message: 'raw server text' },
        }),
        { status: 403, headers: { 'content-type': 'application/json' } },
      ),
    );

    mount(() => (
      <TierBadge api={api} branch="MAIN" customerId="c-1" refreshKey={0} />
    ));

    await sleep();

    expect(host.textContent).toContain(t('error.roleNotAllowed'));
    expect(host.textContent).not.toContain('raw server text');
    expect(host.querySelector('[role="alert"]')).toBeNull();
    expect(host.querySelector('.desk-notice')).not.toBeNull();
  });
});
