import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { errorText, setLang } from '../../../shared/i18n';
import { makeStaffFloorView, makeStaffMeView } from '../../../shared/testing/fixtures';
import type { StaffCtx } from '../cards';
import { HoldsCard } from './HoldsCard';

const BRANCH = 'MAIN';

function me(permissions: string[]) {
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

function makeCtx(perms = ['CustomerHoldPlace', 'VerificationReview']): StaffCtx {
  return {
    me: me(perms),
    floor: floor(),
    refresh: vi.fn(async () => {}),
  } as unknown as StaffCtx;
}

describe('HoldsCard rework tests (#1691: worded errors on holds and customer search)', () => {
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
    vi.restoreAllMocks();
  });

  // ── Row 1: HoldsCard.tsx:74 (loadHolds) ──────────────────────────────────

  it('row 1 (HoldsCard.tsx:74): failed loadHolds shows worded error for 500 network failure', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_holds')) {
          return new Response('Internal Server Error', {
            status: 500,
            statusText: 'Internal Server Error',
            headers: { 'content-type': 'text/plain' },
          });
        }
        return new Response('Not found', { status: 404 });
      }),
    );

    const ctx = makeCtx();
    dispose = render(() => <HoldsCard ctx={ctx} />, host);

    await vi.waitFor(() => {
      const errEl = host.querySelector('[data-testid="holds-error"]');
      expect(errEl, 'holds-error must be rendered when loadHolds fails').not.toBeNull();
      expect(errEl?.textContent).toBe(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
    });
  });

  it('row 1 (HoldsCard.tsx:74): failed loadHolds shows worded error for known refusal code', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_holds')) {
          return new Response(
            JSON.stringify({
              error: {
                code: 'roleNotAllowed',
                message: 'role not permitted',
              },
            }),
            {
              status: 403,
              headers: { 'content-type': 'application/json' },
            },
          );
        }
        return new Response('Not found', { status: 404 });
      }),
    );

    const ctx = makeCtx();
    dispose = render(() => <HoldsCard ctx={ctx} />, host);

    await vi.waitFor(() => {
      const errEl = host.querySelector('[data-testid="holds-error"]');
      expect(errEl, 'holds-error must be rendered when loadHolds is refused').not.toBeNull();
      expect(errEl?.textContent).toBe(errorText('roleNotAllowed', ''));
    });
  });

  it('row 1 (HoldsCard.tsx:74): failed loadHolds shows generic worded message for unknown refusal code (never raw code or message)', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_holds')) {
          return new Response(
            JSON.stringify({
              error: {
                code: 'unknownHoldCode',
                message: 'backend holds failure details',
              },
            }),
            {
              status: 400,
              headers: { 'content-type': 'application/json' },
            },
          );
        }
        return new Response('Not found', { status: 404 });
      }),
    );

    const ctx = makeCtx();
    dispose = render(() => <HoldsCard ctx={ctx} />, host);

    await vi.waitFor(() => {
      const errEl = host.querySelector('[data-testid="holds-error"]');
      expect(errEl, 'holds-error must be rendered on unknown refusal code').not.toBeNull();
      expect(errEl?.textContent).toBe('backend holds failure details');
      expect(errEl?.textContent).not.toContain('unknownHoldCode');
    });
  });

  // ── Row 2: HoldsCard.tsx:96 (handleFind) ──────────────────────────────────

  it('row 2 (HoldsCard.tsx:96): failed customer search shows worded error for 500 network failure, distinct from 0 matches', async () => {
    let customerCallCount = 0;
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_holds')) {
          return new Response(
            JSON.stringify({ version: 1, changed: false, data: [] }),
            { status: 200, headers: { 'content-type': 'application/json' } },
          );
        }
        if (url.includes('/api/v3/views/staff_customers')) {
          customerCallCount++;
          return new Response('Internal Server Error', {
            status: 500,
            statusText: 'Internal Server Error',
            headers: { 'content-type': 'text/plain' },
          });
        }
        return new Response('Not found', { status: 404 });
      }),
    );

    const ctx = makeCtx();
    dispose = render(() => <HoldsCard ctx={ctx} />, host);

    const input = host.querySelector<HTMLInputElement>('[data-testid="holds-key"]')!;
    expect(input).not.toBeNull();
    input.value = 'Somchai';
    input.dispatchEvent(new Event('input', { bubbles: true }));

    const findBtn = host.querySelector<HTMLButtonElement>('[data-testid="holds-find"]')!;
    expect(findBtn).not.toBeNull();
    findBtn.click();

    await vi.waitFor(() => {
      expect(customerCallCount).toBeGreaterThan(0);
      const errEl = host.querySelector('[data-testid="holds-error"]');
      expect(errEl, 'holds-error must be rendered when customer search fails').not.toBeNull();
      expect(errEl?.textContent).toBe(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
    });
  });

  it('row 2 (HoldsCard.tsx:96): failed customer search shows worded error for known refusal code', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_holds')) {
          return new Response(
            JSON.stringify({ version: 1, changed: false, data: [] }),
            { status: 200, headers: { 'content-type': 'application/json' } },
          );
        }
        if (url.includes('/api/v3/views/staff_customers')) {
          return new Response(
            JSON.stringify({
              error: {
                code: 'roleNotAllowed',
                message: 'role not permitted',
              },
            }),
            {
              status: 403,
              headers: { 'content-type': 'application/json' },
            },
          );
        }
        return new Response('Not found', { status: 404 });
      }),
    );

    const ctx = makeCtx();
    dispose = render(() => <HoldsCard ctx={ctx} />, host);

    const input = host.querySelector<HTMLInputElement>('[data-testid="holds-key"]')!;
    input.value = 'Somchai';
    input.dispatchEvent(new Event('input', { bubbles: true }));

    const findBtn = host.querySelector<HTMLButtonElement>('[data-testid="holds-find"]')!;
    findBtn.click();

    await vi.waitFor(() => {
      const errEl = host.querySelector('[data-testid="holds-error"]');
      expect(errEl, 'holds-error must be rendered when customer search is refused').not.toBeNull();
      expect(errEl?.textContent).toBe(errorText('roleNotAllowed', ''));
    });
  });

  it('row 2 (HoldsCard.tsx:96): failed customer search shows generic worded message for unknown refusal code (never raw code or message)', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_holds')) {
          return new Response(
            JSON.stringify({ version: 1, changed: false, data: [] }),
            { status: 200, headers: { 'content-type': 'application/json' } },
          );
        }
        if (url.includes('/api/v3/views/staff_customers')) {
          return new Response(
            JSON.stringify({
              error: {
                code: 'unknownSearchCode',
                message: 'search indexing exploded',
              },
            }),
            {
              status: 400,
              headers: { 'content-type': 'application/json' },
            },
          );
        }
        return new Response('Not found', { status: 404 });
      }),
    );

    const ctx = makeCtx();
    dispose = render(() => <HoldsCard ctx={ctx} />, host);

    const input = host.querySelector<HTMLInputElement>('[data-testid="holds-key"]')!;
    input.value = 'Somchai';
    input.dispatchEvent(new Event('input', { bubbles: true }));

    const findBtn = host.querySelector<HTMLButtonElement>('[data-testid="holds-find"]')!;
    findBtn.click();

    await vi.waitFor(() => {
      const errEl = host.querySelector('[data-testid="holds-error"]');
      expect(errEl, 'holds-error must be rendered on unknown refusal code in search').not.toBeNull();
      expect(errEl?.textContent).toBe('search indexing exploded');
      expect(errEl?.textContent).not.toContain('unknownSearchCode');
    });
  });

  it('row 2 (HoldsCard.tsx:96): successful customer search with 0 matches shows no error (distinct from failure)', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_holds')) {
          return new Response(
            JSON.stringify({ version: 1, changed: false, data: [] }),
            { status: 200, headers: { 'content-type': 'application/json' } },
          );
        }
        if (url.includes('/api/v3/views/staff_customers')) {
          return new Response(
            JSON.stringify({ version: 1, changed: false, data: [] }),
            { status: 200, headers: { 'content-type': 'application/json' } },
          );
        }
        return new Response('Not found', { status: 404 });
      }),
    );

    const ctx = makeCtx();
    dispose = render(() => <HoldsCard ctx={ctx} />, host);

    const input = host.querySelector<HTMLInputElement>('[data-testid="holds-key"]')!;
    input.value = 'UnknownPerson';
    input.dispatchEvent(new Event('input', { bubbles: true }));

    const findBtn = host.querySelector<HTMLButtonElement>('[data-testid="holds-find"]')!;
    findBtn.click();

    // Give time for search to complete
    await new Promise((r) => setTimeout(r, 50));
    const errEl = host.querySelector('[data-testid="holds-error"]');
    expect(errEl, 'no error should be shown when 0 results match').toBeNull();
    const customerEls = host.querySelectorAll('[data-customer-id]');
    expect(customerEls).toHaveLength(0);
  });
});
