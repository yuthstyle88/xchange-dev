// #4792: Staff holds Place and Release send actor branch in request body.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import {
  makeStaffCustomerRow,
  makeStaffFloorView,
  makeStaffHoldRow,
  makeStaffMeView,
} from '../../../shared/testing/fixtures';
import type { StaffCtx } from '../cards';
import { HoldsCard } from './HoldsCard';

const BRANCH = 'BRANCH-4792';

function me(permissions = ['CustomerHoldPlace', 'VerificationReview']) {
  return makeStaffMeView({
    userId: '00000000-0000-4000-8000-0000000000c1',
    name: 'Ploy',
    roles: [],
    branchId: BRANCH,
    branchName: 'XC-Branch-4792',
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

const CUST = makeStaffCustomerRow({ id: 'CX-4792', displayName: 'Customer 4792' });
const HOLD_OPEN = makeStaffHoldRow({
  id: 'h-4792-open',
  customerId: 'CX-4792',
  customerName: 'Customer 4792',
  reason: 'Suspected AML',
  status: 'OPEN',
  at: '2026-10-22T08:00:00Z',
});

describe('HoldsCard branch in request body (#4792)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
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

  it('Place sends request body containing actor branch', async () => {
    const posts: { url: string; body: Record<string, unknown> }[] = [];

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const method = (init?.method ?? 'GET').toUpperCase();

        if (method === 'GET' && url.includes('/api/v3/views/staff_holds')) {
          return json(200, { version: 1, changed: true, data: [] });
        }
        if (method === 'GET' && url.includes('/api/v3/views/staff_customers')) {
          return json(200, { version: 1, changed: true, data: [CUST] });
        }
        if (method === 'POST' && url.includes('/api/customer-holds')) {
          posts.push({ url, body: JSON.parse(String(init?.body ?? '{}')) });
          return json(200, { id: 'h-new-4792' });
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }),
    );

    const ctx = makeCtx(['CustomerHoldPlace']);
    dispose = render(() => <HoldsCard ctx={ctx} />, host);
    await flush();

    const keyInput = host.querySelector<HTMLInputElement>('[data-testid="holds-key"]')!;
    const findBtn = host.querySelector<HTMLButtonElement>('[data-testid="holds-find"]')!;
    const reasonInput = host.querySelector<HTMLInputElement>('[data-testid="holds-reason"]')!;
    const placeBtn = host.querySelector<HTMLButtonElement>('[data-testid="holds-place"]')!;

    typeInto(keyInput, 'Customer');
    findBtn.click();
    await flush();

    const custRow = host.querySelector<HTMLElement>(`[data-customer-id="${CUST.id}"]`)!;
    custRow.click();
    await flush();

    typeInto(reasonInput, 'Testing branch requirement');
    placeBtn.click();
    await flush();

    expect(posts).toHaveLength(1);
    expect(posts[0]!.url).toBe('/api/customer-holds');
    expect(posts[0]!.body).toEqual({
      branch: BRANCH,
      customerId: CUST.id,
      reason: 'Testing branch requirement',
    });
  });

  it('Release sends request body containing actor branch', async () => {
    const posts: { url: string; body: Record<string, unknown> }[] = [];

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const method = (init?.method ?? 'GET').toUpperCase();

        if (method === 'GET' && url.includes('/api/v3/views/staff_holds')) {
          return json(200, { version: 1, changed: true, data: [HOLD_OPEN] });
        }
        if (method === 'POST' && url.includes(`/api/customer-holds/${HOLD_OPEN.id}/release`)) {
          posts.push({ url, body: JSON.parse(String(init?.body ?? '{}')) });
          return json(200, { success: true });
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }),
    );

    const ctx = makeCtx(['VerificationReview']);
    dispose = render(() => <HoldsCard ctx={ctx} />, host);
    await flush();

    const releaseBtn = host.querySelector<HTMLButtonElement>(
      `[data-hold-id="${HOLD_OPEN.id}"] [data-testid="holds-release"]`,
    )!;
    expect(releaseBtn).not.toBeNull();
    releaseBtn.click();
    await flush();

    const dialog = host.querySelector('dialog')!;
    expect(dialog).not.toBeNull();
    typeInto(dialog.querySelector('input')!, 'docs checked');

    const confirmBtn = [...dialog.querySelectorAll('button')].find(
      (b) => b.textContent === 'ยืนยัน' || b.textContent === 'Confirm',
    )!;
    confirmBtn.click();
    await flush();

    expect(posts).toHaveLength(1);
    expect(posts[0]!.url).toBe(`/api/customer-holds/${HOLD_OPEN.id}/release`);
    expect(posts[0]!.body).toEqual({
      branch: BRANCH,
      id: HOLD_OPEN.id,
      note: 'docs checked',
    });
  });
});
