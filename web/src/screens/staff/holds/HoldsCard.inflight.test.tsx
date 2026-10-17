// #2406: Staff holds in-flight guard on Place and release Confirm (§8, §9).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { errorText, setLang } from '../../../shared/i18n';
import type { StaffCustomerRow } from '../../../shared/api/generated/staff_customers';
import type { StaffFloorView } from '../../../shared/api/generated/staff_floor';
import type { StaffHoldRow } from '../../../shared/api/generated/staff_holds';
import type { StaffMeView } from '../../../shared/api/generated/staff_me';
import type { StaffCtx } from '../cards';
import { th as staffTh } from '../i18n';
import { HoldsCard } from './HoldsCard';
import { th } from './i18n';

const BRANCH = 'MAIN';
const H_OPEN = 'h-open-1';

const CUST_A: StaffCustomerRow = { id: 'CX-00000001', displayName: 'Somchai Jaidee' };
const CUST_B: StaffCustomerRow = { id: 'CX-00000007', displayName: 'Somchai Jaiyen' };

const HOLDS: StaffHoldRow[] = [
  {
    id: H_OPEN,
    customerId: CUST_A.id,
    customerName: CUST_A.displayName,
    reason: 'AML check',
    status: 'OPEN',
    at: '2026-10-11T02:00:00Z',
  },
];

function me(permissions = ['CustomerHoldPlace', 'VerificationReview']): StaffMeView {
  return {
    userId: '00000000-0000-4000-8000-0000000000c1',
    name: 'Ploy',
    roles: [],
    branchId: BRANCH,
    branchName: 'XC-B',
    permissions,
  };
}

function floor(): StaffFloorView {
  return {
    phase: 'OPEN',
    hours: null,
    myClockIn: '2026-10-11T01:00:00Z',
    myShift: null,
    cutoff: null,
    target: null,
    safe: null,
  };
}

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function viewResponse(data: unknown): Response {
  return jsonResponse(200, { version: 1, changed: true, data });
}

async function flush(): Promise<void> {
  for (let i = 0; i < 8; i += 1) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

function typeInto(el: HTMLInputElement, value: string): void {
  el.value = value;
  el.dispatchEvent(new Event('input', { bubbles: true }));
}

interface SentPost {
  path: string;
  key: string | null;
  body: string | undefined;
}

describe('HoldsCard in-flight guard (#2406 §8, §9)', () => {
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
    setLang('th');
  });

  async function pickCustomerAndReason(customerId: string, reasonText: string): Promise<void> {
    const keyInput = host.querySelector<HTMLInputElement>('[data-testid="holds-key"]')!;
    const findBtn = host.querySelector<HTMLButtonElement>('[data-testid="holds-find"]')!;
    const reasonInput = host.querySelector<HTMLInputElement>('[data-testid="holds-reason"]')!;

    typeInto(keyInput, 'Somchai');
    findBtn.click();
    await flush();

    const row = host.querySelector<HTMLElement>(`[data-customer-id="${customerId}"]`)!;
    expect(row).not.toBeNull();
    row.click();
    await flush();

    typeInto(reasonInput, reasonText);
  }

  it('row 1: Double click Place -> One POST; no error', async () => {
    const posts: SentPost[] = [];
    let releaseFirstPlace: (() => void) | undefined;

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const path = rawUrl.split('?')[0]!;
        const method = (init?.method ?? 'GET').toUpperCase();

        if (method === 'GET' && path === '/api/v3/views/staff_holds') {
          return viewResponse(HOLDS);
        }
        if (method === 'GET' && path === '/api/v3/views/staff_customers') {
          return viewResponse([CUST_A, CUST_B]);
        }
        if (method === 'POST' && path === '/api/customer-holds') {
          const headers = new Headers(init?.headers);
          posts.push({
            path,
            key: headers.get('idempotency-key'),
            body: init?.body as string | undefined,
          });
          if (posts.length === 1) {
            await new Promise<void>((r) => {
              releaseFirstPlace = r;
            });
            return jsonResponse(200, { id: 'h-new' });
          }
          return jsonResponse(409, {
            error: { code: 'idempotencyConflict', message: 'Request already in progress' },
          });
        }
        return jsonResponse(404, { error: { code: 'notFound', message: 'not found' } });
      }),
    );

    const ctx: StaffCtx = {
      me: me(['CustomerHoldPlace']),
      floor: floor(),
      refresh: vi.fn(async () => {}),
    };
    dispose = render(() => <HoldsCard ctx={ctx} />, host);
    await flush();

    await pickCustomerAndReason(CUST_B.id, 'Large cash');

    const placeBtn = host.querySelector<HTMLButtonElement>('[data-testid="holds-place"]')!;
    placeBtn.click();
    placeBtn.click();
    await flush();

    expect(posts).toHaveLength(1);
    expect(placeBtn.disabled).toBe(true);

    releaseFirstPlace!();
    await flush();

    expect(posts).toHaveLength(1);
    expect(host.querySelector('[data-testid="holds-error"]')).toBeNull();
    expect(host.querySelector('[data-testid="holds-notice"]')?.textContent).toBe(th.saved);
    expect(placeBtn.disabled).toBe(false);
  });

  it('row 2: Double click release Confirm -> One POST; no error', async () => {
    const posts: SentPost[] = [];
    let releaseFirstConfirm: (() => void) | undefined;

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const path = rawUrl.split('?')[0]!;
        const method = (init?.method ?? 'GET').toUpperCase();

        if (method === 'GET' && path === '/api/v3/views/staff_holds') {
          return viewResponse(HOLDS);
        }
        if (method === 'POST' && path === `/api/customer-holds/${H_OPEN}/release`) {
          const headers = new Headers(init?.headers);
          posts.push({
            path,
            key: headers.get('idempotency-key'),
            body: init?.body as string | undefined,
          });
          if (posts.length === 1) {
            await new Promise<void>((r) => {
              releaseFirstConfirm = r;
            });
            return jsonResponse(200, { success: true });
          }
          return jsonResponse(409, {
            error: { code: 'idempotencyConflict', message: 'Request already in progress' },
          });
        }
        return jsonResponse(404, { error: { code: 'notFound', message: 'not found' } });
      }),
    );

    const ctx: StaffCtx = {
      me: me(['VerificationReview']),
      floor: floor(),
      refresh: vi.fn(async () => {}),
    };
    dispose = render(() => <HoldsCard ctx={ctx} />, host);
    await flush();

    const releaseBtn = host.querySelector<HTMLButtonElement>(
      `[data-hold-id="${H_OPEN}"] [data-testid="holds-release"]`,
    )!;
    expect(releaseBtn).not.toBeNull();
    releaseBtn.click();
    await flush();

    const dialog = host.querySelector('dialog')!;
    expect(dialog).not.toBeNull();
    typeInto(dialog.querySelector('input')!, 'documents verified');

    const confirmBtn = [...dialog.querySelectorAll('button')].find(
      (b) => b.textContent === staffTh.confirm,
    )!;
    confirmBtn.click();
    confirmBtn.click();
    await flush();

    expect(posts).toHaveLength(1);

    releaseFirstConfirm!();
    await flush();

    expect(posts).toHaveLength(1);
    expect(host.querySelector('[data-testid="holds-error"]')).toBeNull();
    expect(host.querySelector('[data-testid="holds-notice"]')?.textContent).toBe(th.saved);
    expect(host.querySelector('dialog')).toBeNull();
  });

  it('row 3: Click between success and the re-read -> No second POST', async () => {
    const posts: SentPost[] = [];
    let releaseRefresh: (() => void) | undefined;

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const path = rawUrl.split('?')[0]!;
        const method = (init?.method ?? 'GET').toUpperCase();

        if (method === 'GET' && path === '/api/v3/views/staff_holds') {
          return viewResponse(HOLDS);
        }
        if (method === 'GET' && path === '/api/v3/views/staff_customers') {
          return viewResponse([CUST_A, CUST_B]);
        }
        if (method === 'POST') {
          const headers = new Headers(init?.headers);
          posts.push({
            path,
            key: headers.get('idempotency-key'),
            body: init?.body as string | undefined,
          });
          return jsonResponse(200, { id: 'h-new' });
        }
        return jsonResponse(404, { error: { code: 'notFound', message: 'not found' } });
      }),
    );

    const ctx: StaffCtx = {
      me: me(['CustomerHoldPlace', 'VerificationReview']),
      floor: floor(),
      refresh: vi.fn(
        () =>
          new Promise<void>((r) => {
            releaseRefresh = r;
          }),
      ),
    };
    dispose = render(() => <HoldsCard ctx={ctx} />, host);
    await flush();

    await pickCustomerAndReason(CUST_A.id, 'AML check');

    const placeBtn = host.querySelector<HTMLButtonElement>('[data-testid="holds-place"]')!;
    placeBtn.click();
    await flush();

    // First POST succeeded, ctx.refresh() is still in flight
    expect(posts).toHaveLength(1);
    expect(ctx.refresh).toHaveBeenCalledTimes(1);
    expect(placeBtn.disabled).toBe(true);

    // Even if customer is re-picked and Place / form submit is triggered before re-read finishes, no second POST goes out
    host.querySelector<HTMLElement>(`[data-customer-id="${CUST_B.id}"]`)!.click();
    typeInto(host.querySelector<HTMLInputElement>('[data-testid="holds-reason"]')!, 'Second attempt');
    placeBtn.click();
    host
      .querySelector<HTMLFormElement>('[data-testid="holds-form"]')!
      .dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await flush();

    expect(posts).toHaveLength(1);

    releaseRefresh!();
    await flush();

    expect(placeBtn.disabled).toBe(false);
    expect(posts).toHaveLength(1);
  });

  it('row 4: Place sends {customerId, reason} and disables Place while in flight (kept, holds.test.tsx:480)', async () => {
    const posts: SentPost[] = [];
    let releasePost: (() => void) | undefined;

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const path = rawUrl.split('?')[0]!;
        const method = (init?.method ?? 'GET').toUpperCase();

        if (method === 'GET' && path === '/api/v3/views/staff_holds') {
          return viewResponse(HOLDS);
        }
        if (method === 'GET' && path === '/api/v3/views/staff_customers') {
          return viewResponse([CUST_A, CUST_B]);
        }
        if (method === 'POST' && path === '/api/customer-holds') {
          const headers = new Headers(init?.headers);
          posts.push({
            path,
            key: headers.get('idempotency-key'),
            body: init?.body as string | undefined,
          });
          await new Promise<void>((r) => {
            releasePost = r;
          });
          return jsonResponse(200, { id: 'h-new' });
        }
        return jsonResponse(404, { error: { code: 'notFound', message: 'not found' } });
      }),
    );

    const ctx: StaffCtx = {
      me: me(['CustomerHoldPlace']),
      floor: floor(),
      refresh: vi.fn(async () => {}),
    };
    dispose = render(() => <HoldsCard ctx={ctx} />, host);
    await flush();

    await pickCustomerAndReason(CUST_B.id, 'Large cash');

    const placeBtn = host.querySelector<HTMLButtonElement>('[data-testid="holds-place"]')!;
    placeBtn.click();
    await flush();

    expect(placeBtn.disabled).toBe(true);
    expect(posts).toHaveLength(1);
    expect(JSON.parse(posts[0]!.body!)).toEqual({
      customerId: CUST_B.id,
      reason: 'Large cash',
    });

    releasePost!();
    await flush();
    expect(placeBtn.disabled).toBe(false);
  });

  it('row 5: Retry after a network error resends the same key and byte-equal body after busy clears (kept, holds.test.tsx:583)', async () => {
    const posts: SentPost[] = [];
    let rejectFirst: ((err: Error) => void) | undefined;

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const path = rawUrl.split('?')[0]!;
        const method = (init?.method ?? 'GET').toUpperCase();

        if (method === 'GET' && path === '/api/v3/views/staff_holds') {
          return viewResponse(HOLDS);
        }
        if (method === 'GET' && path === '/api/v3/views/staff_customers') {
          return viewResponse([CUST_A, CUST_B]);
        }
        if (method === 'POST' && path === '/api/customer-holds') {
          const headers = new Headers(init?.headers);
          posts.push({
            path,
            key: headers.get('idempotency-key'),
            body: init?.body as string | undefined,
          });
          if (posts.length === 1) {
            return new Promise<Response>((_, reject) => {
              rejectFirst = reject;
            });
          }
          return jsonResponse(200, { id: 'h-new' });
        }
        return jsonResponse(404, { error: { code: 'notFound', message: 'not found' } });
      }),
    );

    const ctx: StaffCtx = {
      me: me(['CustomerHoldPlace']),
      floor: floor(),
      refresh: vi.fn(async () => {}),
    };
    dispose = render(() => <HoldsCard ctx={ctx} />, host);
    await flush();

    await pickCustomerAndReason(CUST_A.id, 'AML');

    const placeBtn = host.querySelector<HTMLButtonElement>('[data-testid="holds-place"]')!;
    placeBtn.click();
    placeBtn.click();
    await flush();

    expect(placeBtn.disabled).toBe(true);
    expect(posts).toHaveLength(1);

    rejectFirst!(new TypeError('Failed to fetch'));
    await flush();

    expect(placeBtn.disabled).toBe(false);

    // Retry after network failure
    placeBtn.click();
    await flush();

    expect(posts).toHaveLength(2);
    expect(posts[0]!.key).toBeTruthy();
    expect(posts[1]!.key).toBe(posts[0]!.key);
    expect(posts[1]!.body).toBe(posts[0]!.body);
  });

  it('edge case (§9): After a refusal, busy is cleared so a retry is possible (kept, holds.test.tsx:556)', async () => {
    const posts: SentPost[] = [];
    let releaseFirstRefusal: (() => void) | undefined;

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const path = rawUrl.split('?')[0]!;
        const method = (init?.method ?? 'GET').toUpperCase();

        if (method === 'GET' && path === '/api/v3/views/staff_holds') {
          return viewResponse(HOLDS);
        }
        if (method === 'GET' && path === '/api/v3/views/staff_customers') {
          return viewResponse([CUST_A, CUST_B]);
        }
        if (method === 'POST' && path === '/api/customer-holds') {
          const headers = new Headers(init?.headers);
          posts.push({
            path,
            key: headers.get('idempotency-key'),
            body: init?.body as string | undefined,
          });
          if (posts.length === 1) {
            await new Promise<void>((r) => {
              releaseFirstRefusal = r;
            });
            return jsonResponse(403, {
              error: { code: 'roleNotAllowedHere', message: 'refused' },
            });
          }
          return jsonResponse(200, { id: 'h-new' });
        }
        return jsonResponse(404, { error: { code: 'notFound', message: 'not found' } });
      }),
    );

    const ctx: StaffCtx = {
      me: me(['CustomerHoldPlace']),
      floor: floor(),
      refresh: vi.fn(async () => {}),
    };
    dispose = render(() => <HoldsCard ctx={ctx} />, host);
    await flush();

    await pickCustomerAndReason(CUST_B.id, 'Large cash');

    const placeBtn = host.querySelector<HTMLButtonElement>('[data-testid="holds-place"]')!;
    placeBtn.click();
    placeBtn.click();
    await flush();

    expect(placeBtn.disabled).toBe(true);
    expect(posts).toHaveLength(1);

    releaseFirstRefusal!();
    await flush();

    expect(placeBtn.disabled).toBe(false);
    expect(host.querySelector('[data-testid="holds-error"]')?.textContent).toBe(
      errorText('roleNotAllowedHere', 'refused'),
    );

    // Retry succeeds after refusal
    placeBtn.click();
    await flush();

    expect(posts).toHaveLength(2);
    expect(JSON.parse(posts[1]!.body!)).toEqual({
      customerId: CUST_B.id,
      reason: 'Large cash',
    });
    expect(host.querySelector('[data-testid="holds-error"]')).toBeNull();
    expect(host.querySelector('[data-testid="holds-notice"]')?.textContent).toBe(th.saved);
  });
});
