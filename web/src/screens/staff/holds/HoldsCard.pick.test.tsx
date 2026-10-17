// #2270: Staff holds customer pick lifecycle, search sequencing, and post-place clearing.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { errorText, setLang } from '../../../shared/i18n';
import type { StaffCustomerRow } from '../../../shared/api/generated/staff_customers';
import type { StaffFloorView } from '../../../shared/api/generated/staff_floor';
import type { StaffHoldRow } from '../../../shared/api/generated/staff_holds';
import type { StaffMeView } from '../../../shared/api/generated/staff_me';
import type { StaffCtx } from '../cards';
import { HoldsCard } from './HoldsCard';

const BRANCH = 'MAIN';

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

const CUST_A: StaffCustomerRow = { id: 'CX-00000001', displayName: 'Somchai Jaidee' };
const CUST_B: StaffCustomerRow = { id: 'CX-00000007', displayName: 'Somsak Jaiyen' };

const SAMPLE_HOLDS: StaffHoldRow[] = [];

describe('HoldsCard customer pick and search lifecycle (#2270 §8 & §9)', () => {
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

  it('row 1: Pick A, search again (or clear key and Find), press Place -> no POST is sent', async () => {
    const posts: { url: string; body: string }[] = [];
    let searchStep = 0;

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const method = (init?.method ?? 'GET').toUpperCase();
        if (method === 'GET' && url.includes('/api/v3/views/staff_holds')) {
          return view(SAMPLE_HOLDS);
        }
        if (method === 'GET' && url.includes('/api/v3/views/staff_customers')) {
          searchStep++;
          return view(searchStep === 1 ? [CUST_A] : [CUST_B]);
        }
        if (method === 'POST' && url.includes('/api/customer-holds')) {
          posts.push({ url, body: String(init?.body ?? '') });
          return json(200, { id: 'h-new' });
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }),
    );

    const ctx = makeCtx();
    dispose = render(() => <HoldsCard ctx={ctx} />, host);
    await flush();

    const keyInput = host.querySelector<HTMLInputElement>('[data-testid="holds-key"]')!;
    const findBtn = host.querySelector<HTMLButtonElement>('[data-testid="holds-find"]')!;
    const reasonInput = host.querySelector<HTMLInputElement>('[data-testid="holds-reason"]')!;
    const placeBtn = host.querySelector<HTMLButtonElement>('[data-testid="holds-place"]')!;

    // Search for A and pick A
    typeInto(keyInput, 'Somchai');
    findBtn.click();
    await flush();

    const rowA = host.querySelector<HTMLElement>(`[data-customer-id="${CUST_A.id}"]`)!;
    expect(rowA).not.toBeNull();
    rowA.click();
    await flush();

    typeInto(reasonInput, 'Suspicious cash');

    // Search again for someone else (B) without picking B
    typeInto(keyInput, 'Somsak');
    findBtn.click();
    await flush();

    expect(host.querySelector(`[data-customer-id="${CUST_A.id}"]`)).toBeNull();
    const rowB = host.querySelector<HTMLElement>(`[data-customer-id="${CUST_B.id}"]`)!;
    expect(rowB).not.toBeNull();

    // Press Place: no POST may be sent
    placeBtn.click();
    await flush();
    expect(posts).toEqual([]);
    expect(rowB.getAttribute('aria-pressed')).toBe('false');

    // Now pick B, then clear search key to empty and click Find: pick is cleared and Place sends nothing
    rowB.click();
    await flush();
    expect(rowB.getAttribute('aria-pressed')).toBe('true');

    typeInto(keyInput, '   ');
    findBtn.click();
    await flush();
    expect(host.querySelectorAll('[data-customer-id]')).toHaveLength(0);

    placeBtn.click();
    await flush();
    expect(posts).toEqual([]);
  });

  it('row 2: Two searches, the first answers last -> the list shows the second search customers (and late error from first is ignored)', async () => {
    let resolveFirst!: (res: Response) => void;
    let rejectThird!: (err: Error) => void;
    let callIdx = 0;

    vi.stubGlobal(
      'fetch',
      vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const method = (init?.method ?? 'GET').toUpperCase();
        if (method === 'GET' && url.includes('/api/v3/views/staff_holds')) {
          return Promise.resolve(view(SAMPLE_HOLDS));
        }
        if (method === 'GET' && url.includes('/api/v3/views/staff_customers')) {
          callIdx++;
          if (callIdx === 1) {
            return new Promise<Response>((r) => {
              resolveFirst = r;
            });
          }
          if (callIdx === 2) {
            return Promise.resolve(view([CUST_B]));
          }
          if (callIdx === 3) {
            return new Promise<Response>((_, rej) => {
              rejectThird = rej;
            });
          }
          return Promise.resolve(view([CUST_A]));
        }
        return Promise.resolve(json(404, { error: { code: 'notFound', message: 'not found' } }));
      }),
    );

    const ctx = makeCtx();
    dispose = render(() => <HoldsCard ctx={ctx} />, host);
    await flush();

    const keyInput = host.querySelector<HTMLInputElement>('[data-testid="holds-key"]')!;
    const findBtn = host.querySelector<HTMLButtonElement>('[data-testid="holds-find"]')!;

    // First search (slow)
    typeInto(keyInput, 'Somchai');
    findBtn.click();

    // Second search (answers immediately with CUST_B)
    typeInto(keyInput, 'Somsak');
    findBtn.click();
    await flush();

    expect(host.querySelector(`[data-customer-id="${CUST_B.id}"]`)).not.toBeNull();
    expect(host.querySelector(`[data-customer-id="${CUST_A.id}"]`)).toBeNull();

    // Now the first search finally resolves with CUST_A: must be ignored
    resolveFirst(view([CUST_A]));
    await flush();

    expect(host.querySelector(`[data-customer-id="${CUST_B.id}"]`)).not.toBeNull();
    expect(host.querySelector(`[data-customer-id="${CUST_A.id}"]`)).toBeNull();

    // Third search (slow error) followed by fourth search (fast success with CUST_A)
    typeInto(keyInput, 'FailSlow');
    findBtn.click();
    typeInto(keyInput, 'Somchai');
    findBtn.click();
    await flush();

    expect(host.querySelector(`[data-customer-id="${CUST_A.id}"]`)).not.toBeNull();
    rejectThird(new TypeError('Failed to fetch'));
    await flush();

    expect(host.querySelector(`[data-customer-id="${CUST_A.id}"]`)).not.toBeNull();
    expect(host.querySelector('[data-testid="holds-error"]')).toBeNull();
  });

  it('row 3: Place succeeds, press Place again -> clears the pick and reason, and sends no second POST', async () => {
    const posts: { customerId: string; reason: string }[] = [];

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
          posts.push(JSON.parse(String(init?.body ?? '{}')));
          return json(200, { id: 'h-new' });
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }),
    );

    const ctx = makeCtx();
    dispose = render(() => <HoldsCard ctx={ctx} />, host);
    await flush();

    const keyInput = host.querySelector<HTMLInputElement>('[data-testid="holds-key"]')!;
    const findBtn = host.querySelector<HTMLButtonElement>('[data-testid="holds-find"]')!;
    const reasonInput = host.querySelector<HTMLInputElement>('[data-testid="holds-reason"]')!;
    const placeBtn = host.querySelector<HTMLButtonElement>('[data-testid="holds-place"]')!;

    typeInto(keyInput, 'Somchai');
    findBtn.click();
    await flush();

    const rowA = host.querySelector<HTMLElement>(`[data-customer-id="${CUST_A.id}"]`)!;
    rowA.click();
    await flush();

    typeInto(reasonInput, 'AML review');
    placeBtn.click();
    await flush();

    expect(posts).toEqual([{ customerId: CUST_A.id, reason: 'AML review' }]);

    // Press Place again: no second POST
    placeBtn.click();
    await flush();
    expect(posts).toHaveLength(1);
    expect(rowA.getAttribute('aria-pressed')).toBe('false');
    expect(reasonInput.value).toBe('');
  });

  it('row 4: No customer picked, press Place -> nothing sent, and no row is marked picked before clicking', async () => {
    const posts: unknown[] = [];

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const method = (init?.method ?? 'GET').toUpperCase();
        if (method === 'GET' && url.includes('/api/v3/views/staff_holds')) {
          return view(SAMPLE_HOLDS);
        }
        if (method === 'GET' && url.includes('/api/v3/views/staff_customers')) {
          return view([CUST_A, CUST_B]);
        }
        if (method === 'POST' && url.includes('/api/customer-holds')) {
          posts.push(init?.body);
          return json(200, { id: 'h-new' });
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }),
    );

    const ctx = makeCtx();
    dispose = render(() => <HoldsCard ctx={ctx} />, host);
    await flush();

    const keyInput = host.querySelector<HTMLInputElement>('[data-testid="holds-key"]')!;
    const findBtn = host.querySelector<HTMLButtonElement>('[data-testid="holds-find"]')!;
    const reasonInput = host.querySelector<HTMLInputElement>('[data-testid="holds-reason"]')!;
    const placeBtn = host.querySelector<HTMLButtonElement>('[data-testid="holds-place"]')!;

    typeInto(keyInput, 'Som');
    findBtn.click();
    await flush();

    typeInto(reasonInput, 'AML check');
    placeBtn.click();
    await flush();
    expect(posts).toEqual([]);

    const rowA = host.querySelector<HTMLElement>(`[data-customer-id="${CUST_A.id}"]`)!;
    const rowB = host.querySelector<HTMLElement>(`[data-customer-id="${CUST_B.id}"]`)!;
    expect(rowA.getAttribute('aria-pressed')).toBe('false');
    expect(rowB.getAttribute('aria-pressed')).toBe('false');
  });

  it('row 5: A refusal keeps the picked customer (marked as picked) and the reason', async () => {
    const posts: { customerId: string; reason: string }[] = [];

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const method = (init?.method ?? 'GET').toUpperCase();
        if (method === 'GET' && url.includes('/api/v3/views/staff_holds')) {
          return view(SAMPLE_HOLDS);
        }
        if (method === 'GET' && url.includes('/api/v3/views/staff_customers')) {
          return view([CUST_A, CUST_B]);
        }
        if (method === 'POST' && url.includes('/api/customer-holds')) {
          posts.push(JSON.parse(String(init?.body ?? '{}')));
          return json(403, { error: { code: 'roleNotAllowedHere', message: 'refused' } });
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }),
    );

    const ctx = makeCtx();
    dispose = render(() => <HoldsCard ctx={ctx} />, host);
    await flush();

    const keyInput = host.querySelector<HTMLInputElement>('[data-testid="holds-key"]')!;
    const findBtn = host.querySelector<HTMLButtonElement>('[data-testid="holds-find"]')!;
    const reasonInput = host.querySelector<HTMLInputElement>('[data-testid="holds-reason"]')!;
    const placeBtn = host.querySelector<HTMLButtonElement>('[data-testid="holds-place"]')!;

    typeInto(keyInput, 'Som');
    findBtn.click();
    await flush();

    const rowA = host.querySelector<HTMLElement>(`[data-customer-id="${CUST_A.id}"]`)!;
    const rowB = host.querySelector<HTMLElement>(`[data-customer-id="${CUST_B.id}"]`)!;
    rowB.click();
    await flush();

    typeInto(reasonInput, 'Large cash');
    placeBtn.click();
    await flush();

    expect(host.querySelector('[data-testid="holds-error"]')?.textContent).toBe(
      errorText('roleNotAllowedHere', 'refused'),
    );
    expect(reasonInput.value).toBe('Large cash');

    placeBtn.click();
    await flush();
    expect(posts).toHaveLength(2);
    expect(posts[1]).toEqual({ customerId: CUST_B.id, reason: 'Large cash' });
    expect(rowB.getAttribute('aria-pressed')).toBe('true');
    expect(rowA.getAttribute('aria-pressed')).toBe('false');
  });

  it('edge case 1: A search that fails clears the pick', async () => {
    const posts: unknown[] = [];
    let searchCalls = 0;

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const method = (init?.method ?? 'GET').toUpperCase();
        if (method === 'GET' && url.includes('/api/v3/views/staff_holds')) {
          return view(SAMPLE_HOLDS);
        }
        if (method === 'GET' && url.includes('/api/v3/views/staff_customers')) {
          searchCalls++;
          if (searchCalls === 1) return view([CUST_A]);
          return json(500, { error: { code: 'serverError', message: 'boom' } });
        }
        if (method === 'POST' && url.includes('/api/customer-holds')) {
          posts.push(init?.body);
          return json(200, { id: 'h-new' });
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }),
    );

    const ctx = makeCtx();
    dispose = render(() => <HoldsCard ctx={ctx} />, host);
    await flush();

    const keyInput = host.querySelector<HTMLInputElement>('[data-testid="holds-key"]')!;
    const findBtn = host.querySelector<HTMLButtonElement>('[data-testid="holds-find"]')!;
    const reasonInput = host.querySelector<HTMLInputElement>('[data-testid="holds-reason"]')!;
    const placeBtn = host.querySelector<HTMLButtonElement>('[data-testid="holds-place"]')!;

    typeInto(keyInput, 'Somchai');
    findBtn.click();
    await flush();

    host.querySelector<HTMLElement>(`[data-customer-id="${CUST_A.id}"]`)!.click();
    await flush();
    typeInto(reasonInput, 'AML');

    // Second search fails
    typeInto(keyInput, 'Other');
    findBtn.click();
    await flush();

    expect(host.querySelector('[data-testid="holds-error"]')).not.toBeNull();
    placeBtn.click();
    await flush();
    expect(posts).toEqual([]);
  });

  it('edge case 2: The same customer appearing in the new search must be picked again before Place sends', async () => {
    const posts: { customerId: string; reason: string }[] = [];

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
          posts.push(JSON.parse(String(init?.body ?? '{}')));
          return json(200, { id: 'h-new' });
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }),
    );

    const ctx = makeCtx();
    dispose = render(() => <HoldsCard ctx={ctx} />, host);
    await flush();

    const keyInput = host.querySelector<HTMLInputElement>('[data-testid="holds-key"]')!;
    const findBtn = host.querySelector<HTMLButtonElement>('[data-testid="holds-find"]')!;
    const reasonInput = host.querySelector<HTMLInputElement>('[data-testid="holds-reason"]')!;
    const placeBtn = host.querySelector<HTMLButtonElement>('[data-testid="holds-place"]')!;

    // First search returns CUST_A, clerk picks CUST_A
    typeInto(keyInput, 'Somchai');
    findBtn.click();
    await flush();

    const rowA1 = host.querySelector<HTMLElement>(`[data-customer-id="${CUST_A.id}"]`)!;
    rowA1.click();
    await flush();

    typeInto(reasonInput, 'Structuring');

    // Second search also returns CUST_A: must not be selected until clicked again
    typeInto(keyInput, 'Somchai J');
    findBtn.click();
    await flush();

    placeBtn.click();
    await flush();
    expect(posts).toEqual([]);

    const rowA2 = host.querySelector<HTMLElement>(`[data-customer-id="${CUST_A.id}"]`)!;
    expect(rowA2.getAttribute('aria-pressed')).toBe('false');

    // Pick CUST_A again in the new search results, then Place sends
    rowA2.click();
    await flush();
    expect(rowA2.getAttribute('aria-pressed')).toBe('true');

    placeBtn.click();
    await flush();
    expect(posts).toEqual([{ customerId: CUST_A.id, reason: 'Structuring' }]);
  });

  it('in-flight search: clicking a row of the old list while the new search request is pending sends no POST (both success and failure variants)', async () => {
    const posts: { customerId: string; reason: string }[] = [];
    let resolvePendingSuccess!: (res: Response) => void;
    let resolvePendingFailure!: (res: Response) => void;
    let searchStep = 0;

    vi.stubGlobal(
      'fetch',
      vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const method = (init?.method ?? 'GET').toUpperCase();
        if (method === 'GET' && url.includes('/api/v3/views/staff_holds')) {
          return Promise.resolve(view(SAMPLE_HOLDS));
        }
        if (method === 'GET' && url.includes('/api/v3/views/staff_customers')) {
          searchStep++;
          if (searchStep === 1) return Promise.resolve(view([CUST_A]));
          if (searchStep === 2) {
            return new Promise<Response>((r) => {
              resolvePendingSuccess = r;
            });
          }
          if (searchStep === 3) return Promise.resolve(view([CUST_A]));
          return new Promise<Response>((r) => {
            resolvePendingFailure = r;
          });
        }
        if (method === 'POST' && url.includes('/api/customer-holds')) {
          posts.push(JSON.parse(String(init?.body ?? '{}')));
          return Promise.resolve(json(200, { id: 'h-new' }));
        }
        return Promise.resolve(json(404, { error: { code: 'notFound', message: 'not found' } }));
      }),
    );

    const ctx = makeCtx();
    dispose = render(() => <HoldsCard ctx={ctx} />, host);
    await flush();

    const keyInput = host.querySelector<HTMLInputElement>('[data-testid="holds-key"]')!;
    const findBtn = host.querySelector<HTMLButtonElement>('[data-testid="holds-find"]')!;
    const reasonInput = host.querySelector<HTMLInputElement>('[data-testid="holds-reason"]')!;
    const placeBtn = host.querySelector<HTMLButtonElement>('[data-testid="holds-place"]')!;

    typeInto(reasonInput, 'AML check');

    // Variant 1 (search-success): search 1 returns [CUST_A]
    typeInto(keyInput, 'Somchai');
    findBtn.click();
    await flush();

    const oldRowA1 = host.querySelector<HTMLElement>(`[data-customer-id="${CUST_A.id}"]`)!;
    expect(oldRowA1).not.toBeNull();

    // Start search 2 (pending); click oldRowA1 while search 2 is still in flight
    typeInto(keyInput, 'Somsak');
    findBtn.click();
    oldRowA1.click();

    // Search 2 resolves with [CUST_B]
    resolvePendingSuccess(view([CUST_B]));
    await flush();

    placeBtn.click();
    await flush();
    expect(posts, 'no POST after success variant replaces list').toEqual([]);

    // Variant 2 (search-failure): search 3 returns [CUST_A]
    typeInto(keyInput, 'Somchai');
    findBtn.click();
    await flush();

    const oldRowA2 = host.querySelector<HTMLElement>(`[data-customer-id="${CUST_A.id}"]`)!;
    expect(oldRowA2).not.toBeNull();

    // Start search 4 (pending); click oldRowA2 while search 4 is still in flight
    typeInto(keyInput, 'FailSearch');
    findBtn.click();
    oldRowA2.click();

    // Search 4 fails with 503
    resolvePendingFailure(json(503, { error: { code: 'serverError', message: 'unavailable' } }));
    await flush();

    placeBtn.click();
    await flush();
    expect(posts, 'no POST after failure variant replaces list').toEqual([]);
  });
});
