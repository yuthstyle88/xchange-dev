import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import {
  makeStaffCustomerRow,
  makeStaffFloorView,
  makeStaffMeView,
} from '../../../shared/testing/fixtures';
import type { StaffCtx } from '../cards';
import { HoldsCard } from './HoldsCard';

const BRANCH = 'MAIN';

function me(permissions = ['CustomerHoldPlace', 'VerificationReview']) {
  return makeStaffMeView({
    userId: '00000000-0000-4000-8000-0000000000c1',
    name: 'Ploy',
    roles: [],
    branchId: BRANCH,
    branchName: 'XC-B',
    permissions,
  });
}

function floor() {
  return makeStaffFloorView({
    phase: 'OPEN',
    hours: null,
    myClockIn: '2026-10-11T01:00:00Z',
    myShift: null,
    cutoff: null,
    target: null,
    safe: null,
  });
}

function makeCtx(perms?: string[]): StaffCtx {
  return {
    me: me(perms),
    floor: floor(),
    refresh: vi.fn(async () => {}),
  } as unknown as StaffCtx;
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function view(data: unknown): Response {
  return json(200, { version: 1, changed: true, data });
}

const CUST_A = makeStaffCustomerRow({ id: 'CX-00000001', displayName: 'Somchai Jaidee' });

const SAMPLE_HOLDS: unknown[] = [];

describe('HoldsCard #2738: Place hold with no customer picked', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let posts: { url: string; body: string }[];

  beforeEach(() => {
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
    posts = [];

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const method = (init?.method ?? 'GET').toUpperCase();
        if (method === 'GET' && url.includes('/api/v3/views/staff_holds')) {
          return view(SAMPLE_HOLDS);
        }
        if (method === 'GET' && url.includes('/api/v3/views/staff_customers')) {
          return view([CUST_A]);
        }
        if (method === 'POST' && url.includes('/api/customer-holds')) {
          posts.push({ url, body: String(init?.body ?? '') });
          return json(200, { id: 'h-new' });
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }),
    );
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    setLang('th');
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  async function flush(): Promise<void> {
    for (let i = 0; i < 8; i++) {
      await new Promise((r) => setTimeout(r, 0));
    }
  }

  function typeInto(el: HTMLInputElement, value: string): void {
    el.value = value;
    el.dispatchEvent(new Event('input', { bubbles: true }));
  }

  it('H1: No customer picked, a reason typed, click Place -> no request sent; holds-error shows pickCustomer; reason kept', async () => {
    const ctx = makeCtx();
    dispose = render(() => <HoldsCard ctx={ctx} />, host);
    await flush();

    const reasonInput = host.querySelector<HTMLInputElement>('[data-testid="holds-reason"]')!;
    const placeBtn = host.querySelector<HTMLButtonElement>('[data-testid="holds-place"]')!;

    typeInto(reasonInput, 'Need ID verification');
    placeBtn.click();
    await flush();

    expect(posts).toHaveLength(0);
    const errEl = host.querySelector<HTMLElement>('[data-testid="holds-error"]');
    expect(errEl).not.toBeNull();
    expect(errEl!.textContent).toBe('กรุณาเลือกลูกค้าจากผลการค้นหาก่อน');
    expect(reasonInput.value).toBe('Need ID verification');
  });

  it('H2: Same, but press Enter / submit the form -> same as H1', async () => {
    const ctx = makeCtx();
    dispose = render(() => <HoldsCard ctx={ctx} />, host);
    await flush();

    const reasonInput = host.querySelector<HTMLInputElement>('[data-testid="holds-reason"]')!;
    const form = host.querySelector<HTMLFormElement>('[data-testid="holds-form"]')!;

    typeInto(reasonInput, 'Suspicious transaction');
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await flush();

    expect(posts).toHaveLength(0);
    const errEl = host.querySelector<HTMLElement>('[data-testid="holds-error"]');
    expect(errEl).not.toBeNull();
    expect(errEl!.textContent).toBe('กรุณาเลือกลูกค้าจากผลการค้นหาก่อน');
    expect(reasonInput.value).toBe('Suspicious transaction');
  });

  it('H3: A hold was just placed ("Saved" shown), then Place again with no customer -> no request; holds-notice gone; holds-error shows pickCustomer', async () => {
    const ctx = makeCtx();
    dispose = render(() => <HoldsCard ctx={ctx} />, host);
    await flush();

    const keyInput = host.querySelector<HTMLInputElement>('[data-testid="holds-key"]')!;
    const findBtn = host.querySelector<HTMLButtonElement>('[data-testid="holds-find"]')!;
    const reasonInput = host.querySelector<HTMLInputElement>('[data-testid="holds-reason"]')!;
    const placeBtn = host.querySelector<HTMLButtonElement>('[data-testid="holds-place"]')!;

    // Search and pick customer A
    typeInto(keyInput, 'Somchai');
    findBtn.click();
    await flush();

    const rowA = host.querySelector<HTMLElement>(`[data-customer-id="${CUST_A.id}"]`)!;
    rowA.click();
    await flush();

    typeInto(reasonInput, 'First hold');
    placeBtn.click();
    await flush();

    // Verify first hold placed successfully
    expect(posts).toHaveLength(1);
    expect(host.querySelector('[data-testid="holds-notice"]')?.textContent).toBe('บันทึกแล้ว');

    // Click Place again without picking a customer
    placeBtn.click();
    await flush();

    expect(posts).toHaveLength(1);
    expect(host.querySelector('[data-testid="holds-notice"]')).toBeNull();
    const errEl = host.querySelector<HTMLElement>('[data-testid="holds-error"]');
    expect(errEl).not.toBeNull();
    expect(errEl!.textContent).toBe('กรุณาเลือกลูกค้าจากผลการค้นหาก่อน');
  });

  it('H4: H1, then pick a customer and Place succeeds -> one POST; holds-error gone; holds-notice = th.saved', async () => {
    const ctx = makeCtx();
    dispose = render(() => <HoldsCard ctx={ctx} />, host);
    await flush();

    const keyInput = host.querySelector<HTMLInputElement>('[data-testid="holds-key"]')!;
    const findBtn = host.querySelector<HTMLButtonElement>('[data-testid="holds-find"]')!;
    const reasonInput = host.querySelector<HTMLInputElement>('[data-testid="holds-reason"]')!;
    const placeBtn = host.querySelector<HTMLButtonElement>('[data-testid="holds-place"]')!;

    // Trigger H1 first: click Place with no customer
    typeInto(reasonInput, 'Hold reason');
    placeBtn.click();
    await flush();

    expect(posts).toHaveLength(0);
    const errEl = host.querySelector<HTMLElement>('[data-testid="holds-error"]');
    expect(errEl).not.toBeNull();
    expect(errEl!.textContent).toBe('กรุณาเลือกลูกค้าจากผลการค้นหาก่อน');

    // Now search and pick customer A
    typeInto(keyInput, 'Somchai');
    findBtn.click();
    await flush();

    const rowA = host.querySelector<HTMLElement>(`[data-customer-id="${CUST_A.id}"]`)!;
    rowA.click();
    await flush();

    // Place succeeds
    placeBtn.click();
    await flush();

    expect(posts).toHaveLength(1);
    expect(host.querySelector('[data-testid="holds-error"]')).toBeNull();
    expect(host.querySelector('[data-testid="holds-notice"]')?.textContent).toBe('บันทึกแล้ว');
  });
});
