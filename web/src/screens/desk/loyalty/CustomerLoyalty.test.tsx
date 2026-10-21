// CustomerLoyalty tests (FE-desk, #4038; joins PhoneVerify and TierBadge).
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import type { ApiClient } from '../../../shared/api/client';
import { setLang } from '../../../shared/i18n';
import type { DeskStore } from '../desk-store';
import { CustomerLoyalty } from './CustomerLoyalty';

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

const el = <T extends Element>(sel: string) => host.querySelector<T>(sel);

function type(sel: string, value: string) {
  const i = el<HTMLInputElement>(sel)!;
  i.value = value;
  i.dispatchEvent(new Event('input', { bubbles: true }));
}

function createMockApi(confirmResult: { verified: boolean; identityId?: string } = { verified: true, identityId: 'id-1' }) {
  let challengeSeq = 0;
  const view = vi.fn(async () => ({
    version: 1,
    changed: false,
    data: { tier: 'GOLD' },
  }));
  const command = vi.fn(async (_method: string, path: string, _body: unknown, _key: string) => {
    if (path.includes('/phone-verifications/') && path.endsWith('/confirm')) {
      return confirmResult;
    }
    if (path.includes('/phone-verifications')) {
      challengeSeq += 1;
      return { challengeId: `ch-${challengeSeq}`, expiresAt: new Date(Date.now() + 300_000).toISOString() };
    }
    throw new Error(`Unexpected command path: ${path}`);
  });
  return {
    view,
    command,
  } as unknown as ApiClient & {
    view: ReturnType<typeof vi.fn>;
    command: ReturnType<typeof vi.fn>;
  };
}

function createMockStore(api: ApiClient, customerId: () => string | undefined, branch = () => 'MAIN'): DeskStore {
  return {
    api,
    branch,
    customerId,
  } as unknown as DeskStore;
}

describe('CustomerLoyalty', () => {
  it('row 1: customerId "" renders empty host and makes no view call', async () => {
    const api = createMockApi();
    const [customerId] = createSignal<string | undefined>('');
    const store = createMockStore(api, customerId);

    mount(() => <CustomerLoyalty store={store} />);
    await sleep();

    expect(host.innerHTML).toBe('');
    expect(api.view).not.toHaveBeenCalled();
  });

  it('row 2: customerId "c-1" views customer_tier with correct branch and customer', async () => {
    const api = createMockApi();
    const [customerId] = createSignal<string | undefined>('c-1');
    const store = createMockStore(api, customerId);

    mount(() => <CustomerLoyalty store={store} />);
    await sleep();

    expect(api.view).toHaveBeenCalledWith(
      'customer_tier',
      expect.objectContaining({
        branch: 'MAIN',
        customer: 'c-1',
      }),
    );
    expect(api.view).toHaveBeenCalledTimes(1);
    // The phone check is part of the panel (ticket row 2), not only the tier badge.
    expect(el('input[name="phone"]')).not.toBeNull();
  });

  it('row 3: send code, confirm answers { verified: true } views customer_tier twice', async () => {
    const api = createMockApi({ verified: true, identityId: 'id-1' });
    const [customerId] = createSignal<string | undefined>('c-1');
    const store = createMockStore(api, customerId);

    mount(() => <CustomerLoyalty store={store} />);
    await sleep();

    expect(api.view).toHaveBeenCalledTimes(1);

    type('input[name="phone"]', '0812345678');
    el<HTMLButtonElement>('button[name="sendCode"]')!.click();
    await sleep();

    type('input[name="code"]', '123456');
    el<HTMLButtonElement>('button[name="confirmCode"]')!.click();
    await sleep();

    expect(api.view).toHaveBeenCalledTimes(2);
  });

  it('row 4: confirm answers { verified: false } views customer_tier once only', async () => {
    const api = createMockApi({ verified: false });
    const [customerId] = createSignal<string | undefined>('c-1');
    const store = createMockStore(api, customerId);

    mount(() => <CustomerLoyalty store={store} />);
    await sleep();

    expect(api.view).toHaveBeenCalledTimes(1);

    type('input[name="phone"]', '0812345678');
    el<HTMLButtonElement>('button[name="sendCode"]')!.click();
    await sleep();

    type('input[name="code"]', '123456');
    el<HTMLButtonElement>('button[name="confirmCode"]')!.click();
    await sleep();

    expect(api.view).toHaveBeenCalledTimes(1);
  });

  it('row 5: customerId changes to "c-2" views customer_tier with c-2', async () => {
    const api = createMockApi();
    const [customerId, setCustomerId] = createSignal<string | undefined>('c-1');
    const store = createMockStore(api, customerId);

    mount(() => <CustomerLoyalty store={store} />);
    await sleep();

    expect(api.view).toHaveBeenCalledWith(
      'customer_tier',
      expect.objectContaining({
        customer: 'c-1',
      }),
    );

    setCustomerId('c-2');
    await sleep();

    expect(api.view).toHaveBeenCalledWith(
      'customer_tier',
      expect.objectContaining({
        customer: 'c-2',
      }),
    );
    expect(api.view).toHaveBeenCalledTimes(2);
  });
});
