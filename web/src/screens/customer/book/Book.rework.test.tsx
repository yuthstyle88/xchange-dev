// Rework tests for APP-16 customer book screen (#985) covering review items 2–5 and controller date rulings.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import { BookScreen } from './Book';

describe('APP-16 PR 1659 rework tests (must-fix items 2–5)', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setLang('th');
    setAudience('customer');
    container = document.createElement('div');
    document.body.appendChild(container);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    container.remove();
    window.history.replaceState({}, '', '/app/book');
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  const SAMPLE_BRANCHES = {
    branches: [
      {
        code: 'XC-B01',
        name: 'สาขาสยามพารากอน',
        timezone: 'Asia/Bangkok',
        bookingEnabled: true,
      },
    ],
  };

  const SAMPLE_SLOTS = {
    data: {
      slots: [
        {
          id: 'slot-101',
          start: '2026-10-15T10:00:00Z',
          end: '2026-10-15T10:30:00Z',
          available: 3,
        },
      ],
    },
  };

  it('must-fix 2 & 3: starts with no slots; on 404/500 slot error shows error with retry (no invented slot)', async () => {
    let slotCalls = 0;
    const fetchMock = vi.fn(async (url: string) => {
      if (url === '/api/public/host') {
        return new Response(JSON.stringify({ org: 'XC' }), { status: 200 });
      }
      if (url.startsWith('/api/public/orgs/XC/branches')) {
        return new Response(JSON.stringify(SAMPLE_BRANCHES), { status: 200 });
      }
      if (url.includes('/api/v3/views/booking_slots')) {
        slotCalls++;
        if (slotCalls === 1) {
          // Fails on initial load
          return new Response(JSON.stringify({ error: 'not found' }), { status: 404 });
        }
        return new Response(JSON.stringify(SAMPLE_SLOTS), { status: 200 });
      }
      return new Response('Not found', { status: 404 });
    });

    dispose = render(() => <BookScreen fetch={fetchMock} />, container);

    await vi.waitFor(() => {
      const branchSel = container.querySelector<HTMLSelectElement>('select[name="branchId"]');
      expect(branchSel).not.toBeNull();
      expect(container.querySelector('option[value="XC-B01"]')).not.toBeNull();
    });

    // Select branch to trigger slot load
    const branchSel = container.querySelector<HTMLSelectElement>('select[name="branchId"]')!;
    branchSel.value = 'XC-B01';
    branchSel.dispatchEvent(new Event('change', { bubbles: true }));

    // Initial failure: no fabricated slots exist
    await vi.waitFor(() => {
      expect(slotCalls).toBeGreaterThanOrEqual(1);
      const slotSelect = container.querySelector<HTMLSelectElement>('select[name="slotId"]');
      expect(slotSelect).not.toBeNull();
      // No bookable option
      const options = container.querySelectorAll('select[name="slotId"] option');
      expect(options.length).toBe(1); // Only the placeholder/empty option
      expect(options[0].textContent).toContain('ไม่มีช่วงเวลาที่เปิดรับจอง');

      // Error message and retry button are shown
      const errorEl = container.querySelector('.xc-book-slots-error');
      expect(errorEl).not.toBeNull();
      expect(errorEl?.textContent).toContain('โหลดข้อมูลไม่สำเร็จ');
    });

    // Verify invented slot ID is nowhere in the document
    expect(container.textContent).not.toContain('00000000-0000-4000-8000-000000000001');

    // Click retry
    const retryBtn = container.querySelector<HTMLButtonElement>('.xc-book-slots-error button');
    expect(retryBtn).not.toBeNull();
    retryBtn!.click();

    // After retry succeeds, slots appear and error disappears
    await vi.waitFor(() => {
      expect(slotCalls).toBeGreaterThanOrEqual(2);
      const slotSelect = container.querySelector<HTMLSelectElement>('select[name="slotId"]');
      const bookableOption = slotSelect?.querySelector('option[value="slot-101"]');
      expect(bookableOption).not.toBeNull();
      expect(container.querySelector('.xc-book-slots-error')).toBeNull();
    });
  });

  it('must-fix 3: empty day shows empty state for slots (noSlots)', async () => {
    const fetchMock = vi.fn(async (url: string) => {
      if (url === '/api/public/host') {
        return new Response(JSON.stringify({ org: 'XC' }), { status: 200 });
      }
      if (url.startsWith('/api/public/orgs/XC/branches')) {
        return new Response(JSON.stringify(SAMPLE_BRANCHES), { status: 200 });
      }
      if (url.includes('/api/v3/views/booking_slots')) {
        return new Response(JSON.stringify({ data: { slots: [] } }), { status: 200 });
      }
      return new Response('Not found', { status: 404 });
    });

    dispose = render(() => <BookScreen fetch={fetchMock} />, container);

    await vi.waitFor(() => {
      const branchSel = container.querySelector<HTMLSelectElement>('select[name="branchId"]');
      expect(branchSel).not.toBeNull();
      expect(container.querySelector('option[value="XC-B01"]')).not.toBeNull();
    });

    const branchSel = container.querySelector<HTMLSelectElement>('select[name="branchId"]')!;
    branchSel.value = 'XC-B01';
    branchSel.dispatchEvent(new Event('change', { bubbles: true }));

    await vi.waitFor(() => {
      const emptyEl = container.querySelector('.xc-book-slots-empty');
      expect(emptyEl).not.toBeNull();
      expect(emptyEl?.textContent).toContain('ไม่มีช่วงเวลาที่เปิดรับจอง');
    });
  });

  it('must-fix 3 & 5: host resolution failure shows error state with retry and NEVER calls /api/public/orgs/XC/branches', async () => {
    let hostCalls = 0;
    const branchCalls: string[] = [];

    const fetchMock = vi.fn(async (url: string) => {
      if (url === '/api/public/host') {
        hostCalls++;
        if (hostCalls === 1) {
          return new Response(JSON.stringify({ error: 'host unreachable' }), { status: 503 });
        }
        return new Response(JSON.stringify({ org: 'REAL_ORG' }), { status: 200 });
      }
      if (url.includes('/branches')) {
        branchCalls.push(url);
        return new Response(JSON.stringify(SAMPLE_BRANCHES), { status: 200 });
      }
      if (url.includes('/booking_slots')) {
        return new Response(JSON.stringify(SAMPLE_SLOTS), { status: 200 });
      }
      return new Response('Not found', { status: 404 });
    });

    dispose = render(() => <BookScreen fetch={fetchMock} />, container);

    // Initial load: host fails
    await vi.waitFor(() => {
      expect(hostCalls).toBeGreaterThanOrEqual(1);
      const branchError = container.querySelector('.xc-book-branches-error');
      expect(branchError).not.toBeNull();
      expect(branchError?.textContent).toContain('โหลดข้อมูลไม่สำเร็จ');
    });

    // Contract: MUST NOT silently fall back to 'XC'
    expect(branchCalls).toHaveLength(0);

    // Click retry
    const retryBtn = container.querySelector<HTMLButtonElement>('.xc-book-branches-error button');
    expect(retryBtn).not.toBeNull();
    retryBtn!.click();

    // After retry succeeds, host resolves to REAL_ORG and branches load
    await vi.waitFor(() => {
      expect(branchCalls).toContain('/api/public/orgs/REAL_ORG/branches');
      expect(container.querySelector('.xc-book-branches-error')).toBeNull();
    });
  });

  it('must-fix 3: empty branches list shows empty state (noBranches)', async () => {
    const fetchMock = vi.fn(async (url: string) => {
      if (url === '/api/public/host') {
        return new Response(JSON.stringify({ org: 'XC' }), { status: 200 });
      }
      if (url.startsWith('/api/public/orgs/XC/branches')) {
        return new Response(JSON.stringify({ branches: [] }), { status: 200 });
      }
      if (url.includes('/booking_slots')) {
        return new Response(JSON.stringify(SAMPLE_SLOTS), { status: 200 });
      }
      return new Response('Not found', { status: 404 });
    });

    dispose = render(() => <BookScreen fetch={fetchMock} />, container);

    await vi.waitFor(() => {
      const emptyEl = container.querySelector('.xc-book-branches-empty');
      expect(emptyEl).not.toBeNull();
      expect(emptyEl?.textContent).toContain('ไม่มีสาขาที่เปิดรับจอง');
    });
  });

  it('must-fix 5: skips preview until a branch is chosen (no XC-B01 fallback)', async () => {
    const previewCalls: any[] = [];
    const fetchMock = vi.fn(async (url: string, init?: RequestInit) => {
      if (url === '/api/public/host') {
        return new Response(JSON.stringify({ org: 'XC' }), { status: 200 });
      }
      if (url.startsWith('/api/public/orgs/XC/branches')) {
        return new Response(JSON.stringify(SAMPLE_BRANCHES), { status: 200 });
      }
      if (url.includes('/booking_slots')) {
        return new Response(JSON.stringify(SAMPLE_SLOTS), { status: 200 });
      }
      if (url === '/api/v3/compute/planner_preview') {
        const body = JSON.parse((init?.body as string) || '{}');
        previewCalls.push(body);
        return new Response(JSON.stringify({ thb: '3500.00' }), { status: 200 });
      }
      return new Response('Not found', { status: 404 });
    });

    dispose = render(() => <BookScreen fetch={fetchMock} />, container);

    await vi.waitFor(() => {
      const branchSel = container.querySelector<HTMLSelectElement>('select[name="branchId"]');
      expect(branchSel?.value).toBe('');
    });

    // Type a quantity with NO branch selected
    const qtyInput = container.querySelector<HTMLInputElement>('input[name="quantity"]')!;
    qtyInput.value = '100';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));

    // Small delay to allow any async preview to trigger if buggy
    await new Promise((r) => setTimeout(r, 50));
    expect(previewCalls).toHaveLength(0);
    expect(container.querySelector('[data-testid="planner-preview"]')).toBeNull();

    // Now select a branch
    const branchSel = container.querySelector<HTMLSelectElement>('select[name="branchId"]')!;
    branchSel.value = 'XC-B01';
    branchSel.dispatchEvent(new Event('change', { bubbles: true }));

    await vi.waitFor(() => {
      expect(previewCalls.length).toBeGreaterThanOrEqual(1);
      expect(previewCalls[0].branch).toBe('XC-B01');
      expect(container.querySelector('[data-testid="planner-preview"]')).not.toBeNull();
    });
  });

  it('must-fix 4: re-reads booking_slots after capacityFull or slotUnavailable', async () => {
    let slotFetchCount = 0;
    let currentAvailable = 2;

    const fetchMock = vi.fn(async (url: string, init?: RequestInit) => {
      if (url === '/api/public/host') {
        return new Response(JSON.stringify({ org: 'XC' }), { status: 200 });
      }
      if (url.startsWith('/api/public/orgs/XC/branches')) {
        return new Response(JSON.stringify(SAMPLE_BRANCHES), { status: 200 });
      }
      if (url.includes('/booking_slots')) {
        slotFetchCount++;
        return new Response(
          JSON.stringify({
            data: {
              slots: [
                {
                  id: 'slot-101',
                  start: '2026-10-15T10:00:00Z',
                  end: '2026-10-15T10:30:00Z',
                  available: currentAvailable,
                },
              ],
            },
          }),
          { status: 200 },
        );
      }
      if (url === '/api/bookings' && init?.method === 'POST') {
        // Slot became full while customer was viewing
        currentAvailable = 0;
        return new Response(
          JSON.stringify({
            error: {
              code: 'capacityFull',
              message: 'ที่นั่งในช่วงเวลานี้เต็มแล้ว',
            },
          }),
          { status: 400 },
        );
      }
      return new Response('Not found', { status: 404 });
    });

    dispose = render(() => <BookScreen fetch={fetchMock} />, container);

    await vi.waitFor(() => {
      const branchSel = container.querySelector<HTMLSelectElement>('select[name="branchId"]');
      expect(branchSel).not.toBeNull();
      expect(container.querySelector('option[value="XC-B01"]')).not.toBeNull();
    });

    const branchSel = container.querySelector<HTMLSelectElement>('select[name="branchId"]')!;
    branchSel.value = 'XC-B01';
    branchSel.dispatchEvent(new Event('change', { bubbles: true }));

    await vi.waitFor(() => {
      const slotOpt = container.querySelector<HTMLOptionElement>('option[value="slot-101"]');
      expect(slotOpt).not.toBeNull();
      expect(slotOpt?.disabled).toBe(false);
    });

    const initialFetchCount = slotFetchCount;

    // Fill slot and quantity and submit
    const slotSel = container.querySelector<HTMLSelectElement>('select[name="slotId"]')!;
    slotSel.value = 'slot-101';
    slotSel.dispatchEvent(new Event('change', { bubbles: true }));

    const qtyInput = container.querySelector<HTMLInputElement>('input[name="quantity"]')!;
    qtyInput.value = '100';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));

    const submitBtn = container.querySelector<HTMLButtonElement>('button[type="submit"]')!;
    submitBtn.click();

    // Verify slot was re-read and updated to 0 available and disabled
    await vi.waitFor(() => {
      expect(slotFetchCount).toBeGreaterThan(initialFetchCount);
      const slotOpt = container.querySelector<HTMLOptionElement>('option[value="slot-101"]');
      expect(slotOpt?.disabled).toBe(true);
      expect(slotOpt?.textContent).toContain('ว่าง 0 ที่');
      expect(container.textContent).toContain('ที่นั่งในช่วงเวลานี้เต็มแล้ว');
    });
  });

  it('must-fix 5: sends branch and date query parameters to booking_slots', async () => {
    const slotUrls: string[] = [];
    const fetchMock = vi.fn(async (url: string) => {
      if (url === '/api/public/host') {
        return new Response(JSON.stringify({ org: 'XC' }), { status: 200 });
      }
      if (url.startsWith('/api/public/orgs/XC/branches')) {
        return new Response(JSON.stringify(SAMPLE_BRANCHES), { status: 200 });
      }
      if (url.includes('/booking_slots')) {
        slotUrls.push(url);
        return new Response(JSON.stringify(SAMPLE_SLOTS), { status: 200 });
      }
      return new Response('Not found', { status: 404 });
    });

    dispose = render(() => <BookScreen fetch={fetchMock} />, container);

    await vi.waitFor(() => {
      const branchOpt = container.querySelector('option[value="XC-B01"]');
      expect(branchOpt).not.toBeNull();
    });

    const branchSel = container.querySelector<HTMLSelectElement>('select[name="branchId"]')!;
    branchSel.value = 'XC-B01';
    branchSel.dispatchEvent(new Event('change', { bubbles: true }));

    await vi.waitFor(() => {
      const callWithBranch = slotUrls.find((u) => u.includes('branch=XC-B01'));
      expect(callWithBranch).toBeDefined();
      expect(callWithBranch).toContain('branch=XC-B01');
      expect(callWithBranch).toContain('date=');
    });
  });

  it('must-fix 5: available count text is localized through i18n in th, en, zh', async () => {
    const fetchMock = vi.fn(async (url: string) => {
      if (url === '/api/public/host') {
        return new Response(JSON.stringify({ org: 'XC' }), { status: 200 });
      }
      if (url.startsWith('/api/public/orgs/XC/branches')) {
        return new Response(JSON.stringify(SAMPLE_BRANCHES), { status: 200 });
      }
      if (url.includes('/booking_slots')) {
        return new Response(JSON.stringify(SAMPLE_SLOTS), { status: 200 });
      }
      return new Response('Not found', { status: 404 });
    });

    const selectBranch = async () => {
      await vi.waitFor(() => {
        expect(container.querySelector('option[value="XC-B01"]')).not.toBeNull();
      });
      const branchSel = container.querySelector<HTMLSelectElement>('select[name="branchId"]')!;
      branchSel.value = 'XC-B01';
      branchSel.dispatchEvent(new Event('change', { bubbles: true }));
    };

    // 1. Thai
    setLang('th');
    dispose = render(() => <BookScreen fetch={fetchMock} />, container);
    await selectBranch();
    await vi.waitFor(() => {
      const opt = container.querySelector('option[value="slot-101"]');
      expect(opt?.textContent).toContain('ว่าง 3 ที่');
    });

    // 2. English
    dispose();
    container.innerHTML = '';
    setLang('en');
    dispose = render(() => <BookScreen fetch={fetchMock} />, container);
    await selectBranch();
    await vi.waitFor(() => {
      const opt = container.querySelector('option[value="slot-101"]');
      expect(opt?.textContent).toContain('3 available');
    });

    // 3. Chinese
    dispose();
    container.innerHTML = '';
    setLang('zh');
    dispose = render(() => <BookScreen fetch={fetchMock} />, container);
    await selectBranch();
    await vi.waitFor(() => {
      const opt = container.querySelector('option[value="slot-101"]');
      expect(opt?.textContent).toContain('剩余 3');
    });
  });

  // ── Controller ruling regression tests (PR 1659 / #985) ──────────────────────

  it('ruling row 1: default date is today in chosen branch timezone; no date and no slot read before branch chosen', async () => {
    vi.setSystemTime(new Date('2026-10-14T18:30:00Z'));
    const slotCalls: string[] = [];
    const BRANCHES_WITH_TZ = {
      branches: [
        {
          code: 'XC-B01',
          name: 'สาขาสยามพารากอน',
          timezone: 'Asia/Bangkok',
          bookingEnabled: true,
        },
      ],
    };

    const fetchMock = vi.fn(async (url: string) => {
      if (url === '/api/public/host') {
        return new Response(JSON.stringify({ org: 'XC' }), { status: 200 });
      }
      if (url.startsWith('/api/public/orgs/XC/branches')) {
        return new Response(JSON.stringify(BRANCHES_WITH_TZ), { status: 200 });
      }
      if (url.includes('/booking_slots')) {
        slotCalls.push(url);
        return new Response(JSON.stringify(SAMPLE_SLOTS), { status: 200 });
      }
      return new Response('Not found', { status: 404 });
    });

    dispose = render(() => <BookScreen fetch={fetchMock} />, container);

    // 1. Before branch is chosen: no date and no slot read
    await vi.waitFor(() => {
      const branchSel = container.querySelector<HTMLSelectElement>('select[name="branchId"]');
      expect(branchSel?.value).toBe('');
      expect(container.querySelector('option[value="XC-B01"]')).not.toBeNull();
    });

    const dateInput = container.querySelector<HTMLInputElement>('input[name="serviceDate"]')!;
    expect(dateInput).not.toBeNull();
    expect(dateInput.value).toBe('');
    expect(dateInput.disabled).toBe(true);
    expect(slotCalls).toHaveLength(0);

    // 2. Select branch
    const branchSel = container.querySelector<HTMLSelectElement>('select[name="branchId"]')!;
    branchSel.value = 'XC-B01';
    branchSel.dispatchEvent(new Event('change', { bubbles: true }));

    await vi.waitFor(() => {
      expect(dateInput.value).toBe('2026-10-15');
      expect(dateInput.min).toBe('2026-10-15');
      expect(dateInput.disabled).toBe(false);
      expect(slotCalls.length).toBeGreaterThanOrEqual(1);
      expect(slotCalls[0]).toContain('branch=XC-B01');
      expect(slotCalls[0]).toContain('date=2026-10-15');
    });
  });

  it('ruling row 2: date control has min = today in branch timezone and no max; changing date re-reads booking_slots', async () => {
    const slotCalls: string[] = [];
    const fetchMock = vi.fn(async (url: string) => {
      if (url === '/api/public/host') {
        return new Response(JSON.stringify({ org: 'XC' }), { status: 200 });
      }
      if (url.startsWith('/api/public/orgs/XC/branches')) {
        return new Response(JSON.stringify(SAMPLE_BRANCHES), { status: 200 });
      }
      if (url.includes('/booking_slots')) {
        slotCalls.push(url);
        return new Response(JSON.stringify(SAMPLE_SLOTS), { status: 200 });
      }
      return new Response('Not found', { status: 404 });
    });

    dispose = render(() => <BookScreen fetch={fetchMock} />, container);

    await vi.waitFor(() => {
      const branchSel = container.querySelector<HTMLSelectElement>('select[name="branchId"]');
      expect(branchSel).not.toBeNull();
      expect(container.querySelector('option[value="XC-B01"]')).not.toBeNull();
    });

    const branchSel = container.querySelector<HTMLSelectElement>('select[name="branchId"]')!;
    branchSel.value = 'XC-B01';
    branchSel.dispatchEvent(new Event('change', { bubbles: true }));

    const expectedToday = new Intl.DateTimeFormat('en-CA', {
      timeZone: 'Asia/Bangkok',
      year: 'numeric',
      month: '2-digit',
      day: '2-digit',
    }).format(new Date());

    const dateInput = container.querySelector<HTMLInputElement>('input[name="serviceDate"]')!;
    await vi.waitFor(() => {
      expect(dateInput.value).toBe(expectedToday);
      expect(dateInput.min).toBe(expectedToday);
      expect(dateInput.getAttribute('max')).toBeNull();
    });

    // Change date
    const newDate = '2029-12-25';
    dateInput.value = newDate;
    dateInput.dispatchEvent(new Event('change', { bubbles: true }));

    await vi.waitFor(() => {
      const callWithNewDate = slotCalls.find((u) => u.includes(`date=${newDate}`));
      expect(callWithNewDate).toBeDefined();
      expect(callWithNewDate).toContain('branch=XC-B01');
    });
  });

  it('ruling row 3: ?date= is allowed when >= today in branch timezone; ignored when earlier than today', async () => {
    const slotCalls: string[] = [];
    const fetchMock = vi.fn(async (url: string) => {
      if (url === '/api/public/host') {
        return new Response(JSON.stringify({ org: 'XC' }), { status: 200 });
      }
      if (url.startsWith('/api/public/orgs/XC/branches')) {
        return new Response(JSON.stringify(SAMPLE_BRANCHES), { status: 200 });
      }
      if (url.includes('/booking_slots')) {
        slotCalls.push(url);
        return new Response(JSON.stringify(SAMPLE_SLOTS), { status: 200 });
      }
      return new Response('Not found', { status: 404 });
    });

    const expectedToday = new Intl.DateTimeFormat('en-CA', {
      timeZone: 'Asia/Bangkok',
      year: 'numeric',
      month: '2-digit',
      day: '2-digit',
    }).format(new Date());

    // Case 1: ?date= in the future is accepted
    window.history.replaceState({}, '', '/app/book?date=2029-06-15');
    dispose = render(() => <BookScreen fetch={fetchMock} />, container);

    await vi.waitFor(() => {
      const branchSel = container.querySelector<HTMLSelectElement>('select[name="branchId"]');
      expect(branchSel).not.toBeNull();
      expect(container.querySelector('option[value="XC-B01"]')).not.toBeNull();
    });

    let branchSel = container.querySelector<HTMLSelectElement>('select[name="branchId"]')!;
    branchSel.value = 'XC-B01';
    branchSel.dispatchEvent(new Event('change', { bubbles: true }));

    let dateInput = container.querySelector<HTMLInputElement>('input[name="serviceDate"]')!;
    await vi.waitFor(() => {
      expect(dateInput.value).toBe('2029-06-15');
      const futureCall = slotCalls.find((u) => u.includes('date=2029-06-15'));
      expect(futureCall).toBeDefined();
    });

    dispose();
    container.innerHTML = '';

    // Case 2: ?date= in the past is ignored (defaults to today in branch timezone)
    window.history.replaceState({}, '', '/app/book?date=2020-01-01');
    dispose = render(() => <BookScreen fetch={fetchMock} />, container);

    await vi.waitFor(() => {
      const bSel = container.querySelector<HTMLSelectElement>('select[name="branchId"]');
      expect(bSel).not.toBeNull();
      expect(container.querySelector('option[value="XC-B01"]')).not.toBeNull();
    });

    branchSel = container.querySelector<HTMLSelectElement>('select[name="branchId"]')!;
    branchSel.value = 'XC-B01';
    branchSel.dispatchEvent(new Event('change', { bubbles: true }));

    dateInput = container.querySelector<HTMLInputElement>('input[name="serviceDate"]')!;
    await vi.waitFor(() => {
      expect(dateInput.value).toBe(expectedToday);
      const pastCall = slotCalls.find((u) => u.includes('date=2020-01-01'));
      expect(pastCall).toBeUndefined();
    });
  });

  it('must-fix 1: no slot read happens before a branch is chosen', async () => {
    const slotCalls: string[] = [];
    const fetchMock = vi.fn(async (url: string) => {
      if (url === '/api/public/host') {
        return new Response(JSON.stringify({ org: 'XC' }), { status: 200 });
      }
      if (url.startsWith('/api/public/orgs/XC/branches')) {
        return new Response(JSON.stringify(SAMPLE_BRANCHES), { status: 200 });
      }
      if (url.includes('/booking_slots')) {
        slotCalls.push(url);
        return new Response(JSON.stringify(SAMPLE_SLOTS), { status: 200 });
      }
      return new Response('Not found', { status: 404 });
    });

    dispose = render(() => <BookScreen fetch={fetchMock} />, container);

    await vi.waitFor(() => {
      const branchSel = container.querySelector<HTMLSelectElement>('select[name="branchId"]');
      expect(branchSel).not.toBeNull();
      expect(container.querySelector('option[value="XC-B01"]')).not.toBeNull();
    });

    // Ensure no slot call was made before a branch is selected
    expect(slotCalls).toHaveLength(0);
  });

  it('must-fix 3: host answer without org shows error state even if branches array is returned', async () => {
    const orgCalls: string[] = [];
    const fetchMock = vi.fn(async (url: string) => {
      if (url === '/api/public/host') {
        return new Response(
          JSON.stringify({
            branches: [{ code: 'XC-B01', name: 'สาขาสยามพารากอน', timezone: 'Asia/Bangkok' }],
          }),
          { status: 200 },
        );
      }
      if (url.startsWith('/api/public/orgs/')) {
        orgCalls.push(url);
        return new Response(
          JSON.stringify({
            branches: [{ code: 'XC-B01', name: 'สาขาสยามพารากอน', timezone: 'Asia/Bangkok' }],
          }),
          { status: 200 },
        );
      }
      return new Response('Not found', { status: 404 });
    });

    dispose = render(() => <BookScreen fetch={fetchMock} />, container);

    await vi.waitFor(() => {
      const errorEl = container.querySelector('.xc-book-branches-error');
      expect(errorEl).not.toBeNull();
      expect(errorEl?.textContent).toContain('โหลดข้อมูลไม่สำเร็จ');
    });

    expect(orgCalls).toHaveLength(0);
  });

  it('must-fix 4: invalid timezone on branch shows error state', async () => {
    const BRANCHES_INVALID_TZ = {
      branches: [
        {
          code: 'XC-INV',
          name: 'สาขาผิดพลาด',
          timezone: 'Invalid/NonExistentZone',
          bookingEnabled: true,
        },
      ],
    };
    const fetchMock = vi.fn(async (url: string) => {
      if (url === '/api/public/host') {
        return new Response(JSON.stringify({ org: 'XC' }), { status: 200 });
      }
      if (url.startsWith('/api/public/orgs/XC/branches')) {
        return new Response(JSON.stringify(BRANCHES_INVALID_TZ), { status: 200 });
      }
      return new Response('Not found', { status: 404 });
    });

    dispose = render(() => <BookScreen fetch={fetchMock} branchId="XC-INV" />, container);

    await vi.waitFor(() => {
      const errorEl = container.querySelector('.xc-book-branches-error');
      expect(errorEl).not.toBeNull();
      expect(errorEl?.textContent).toContain('โหลดข้อมูลไม่สำเร็จ');
    });
  });

  it('must-fix 4: selecting a branch with invalid timezone shows error state', async () => {
    const BRANCHES_MIXED = {
      branches: [
        {
          code: 'XC-OK',
          name: 'สาขาปกติ',
          timezone: 'Asia/Bangkok',
          bookingEnabled: true,
        },
        {
          code: 'XC-BAD',
          name: 'สาขาผิด',
          timezone: 'Invalid/NonExistentZone',
          bookingEnabled: true,
        },
      ],
    };
    const fetchMock = vi.fn(async (url: string) => {
      if (url === '/api/public/host') {
        return new Response(JSON.stringify({ org: 'XC' }), { status: 200 });
      }
      if (url.startsWith('/api/public/orgs/XC/branches')) {
        return new Response(JSON.stringify(BRANCHES_MIXED), { status: 200 });
      }
      if (url.includes('/booking_slots')) {
        return new Response(JSON.stringify(SAMPLE_SLOTS), { status: 200 });
      }
      return new Response('Not found', { status: 404 });
    });

    dispose = render(() => <BookScreen fetch={fetchMock} />, container);

    await vi.waitFor(() => {
      const bSel = container.querySelector<HTMLSelectElement>('select[name="branchId"]');
      expect(bSel).not.toBeNull();
    });

    const bSel = container.querySelector<HTMLSelectElement>('select[name="branchId"]')!;
    bSel.value = 'XC-BAD';
    bSel.dispatchEvent(new Event('change', { bubbles: true }));

    await vi.waitFor(() => {
      const errorEl = container.querySelector('.xc-book-branches-error');
      expect(errorEl).not.toBeNull();
    });
  });

  it('guards slot select against mutation E (hidden loading option and select value patch)', async () => {
    let resolveSlots: (res: Response) => void;
    const slotsPromise = new Promise<Response>((resolve) => {
      resolveSlots = resolve;
    });

    const unknownSlotId = '00000000-0000-4000-8000-0000000000ff';
    const createCalls: any[] = [];
    const fetchMock = vi.fn(async (url: string, init?: RequestInit) => {
      if (url === '/api/public/host') {
        return new Response(JSON.stringify({ org: 'XC' }), { status: 200 });
      }
      if (url.startsWith('/api/public/orgs/XC/branches')) {
        return new Response(JSON.stringify(SAMPLE_BRANCHES), { status: 200 });
      }
      if (url.includes('/booking_slots')) {
        return slotsPromise;
      }
      if (url === '/api/bookings' && init?.method === 'POST') {
        const body = JSON.parse(String(init.body));
        createCalls.push(body);
        return new Response(JSON.stringify({ id: 'bk-test-1' }), { status: 200 });
      }
      return new Response('Not found', { status: 404 });
    });

    dispose = render(() => <BookScreen fetch={fetchMock} />, container);

    await vi.waitFor(() => {
      const branchSel = container.querySelector<HTMLSelectElement>('select[name="branchId"]');
      expect(branchSel).not.toBeNull();
      expect(container.querySelector('option[value="XC-B01"]')).not.toBeNull();
    });

    // Choose branch -> triggers slot read
    const branchSel = container.querySelector<HTMLSelectElement>('select[name="branchId"]')!;
    branchSel.value = 'XC-B01';
    branchSel.dispatchEvent(new Event('change', { bubbles: true }));

    // Before slots answer: assert select has no hidden "loading" option
    const loadingOpt = container.querySelector('select[name="slotId"] option[value="loading"]');
    expect(loadingOpt, 'select must have no "loading" option').toBeNull();

    // Before slots answer: attempt to read or set select value to unknown ID
    const slotSel = container.querySelector<HTMLSelectElement>('select[name="slotId"]')!;
    slotSel.value = unknownSlotId;
    slotSel.dispatchEvent(new Event('change', { bubbles: true }));
    expect(slotSel.value).not.toBe(unknownSlotId);

    // Answer slots
    resolveSlots!(new Response(JSON.stringify(SAMPLE_SLOTS), { status: 200 }));

    // Wait for real slot option
    await vi.waitFor(() => {
      const slotOpt = container.querySelector<HTMLOptionElement>('option[value="slot-101"]');
      expect(slotOpt).not.toBeNull();
    });

    // Try setting an unknown slot id not returned by booking_slots
    slotSel.value = unknownSlotId;
    slotSel.dispatchEvent(new Event('change', { bubbles: true }));

    // Submit form
    const submitBtn = container.querySelector<HTMLButtonElement>('button[type="submit"]')!;
    submitBtn.click();

    // Assert that no POST carries a slotId outside SAMPLE_SLOTS
    await vi.waitFor(() => {
      expect(createCalls.length).toBeGreaterThan(0);
    });

    const allowedSlotIds = SAMPLE_SLOTS.data.slots.map((s) => s.id);
    for (const call of createCalls) {
      expect(call.slotId).not.toBe(unknownSlotId);
      if (call.slotId) {
        expect(allowedSlotIds).toContain(call.slotId);
      }
    }
  });

  it('pins slot option label formatting: start 2026-10-14T23:00:00+00:00 renders 06:00 in Asia/Bangkok', async () => {
    const BRANCHES_BANGKOK = {
      branches: [
        {
          code: 'XC-BKK',
          name: 'สาขากรุงเทพ',
          timezone: 'Asia/Bangkok',
          bookingEnabled: true,
        },
      ],
    };
    const SLOTS_BANGKOK = {
      data: {
        slots: [
          {
            id: 'slot-bkk-early',
            start: '2026-10-14T23:00:00+00:00',
            end: '2026-10-14T23:30:00+00:00',
            available: 2,
          },
        ],
      },
    };

    const fetchMock = vi.fn(async (url: string) => {
      if (url === '/api/public/host') {
        return new Response(JSON.stringify({ org: 'XC' }), { status: 200 });
      }
      if (url.startsWith('/api/public/orgs/XC/branches')) {
        return new Response(JSON.stringify(BRANCHES_BANGKOK), { status: 200 });
      }
      if (url.includes('/booking_slots')) {
        return new Response(JSON.stringify(SLOTS_BANGKOK), { status: 200 });
      }
      return new Response('Not found', { status: 404 });
    });

    dispose = render(() => <BookScreen fetch={fetchMock} />, container);

    await vi.waitFor(() => {
      const branchSel = container.querySelector<HTMLSelectElement>('select[name="branchId"]');
      expect(branchSel).not.toBeNull();
      expect(container.querySelector('option[value="XC-BKK"]')).not.toBeNull();
    });

    const branchSel = container.querySelector<HTMLSelectElement>('select[name="branchId"]')!;
    branchSel.value = 'XC-BKK';
    branchSel.dispatchEvent(new Event('change', { bubbles: true }));

    await vi.waitFor(() => {
      const slotOpt = container.querySelector<HTMLOptionElement>('option[value="slot-bkk-early"]');
      expect(slotOpt).not.toBeNull();
      expect(slotOpt!.textContent).toContain('06:00');
    });

    const slotOpt = container.querySelector<HTMLOptionElement>('option[value="slot-bkk-early"]')!;
    expect(slotOpt.textContent).toContain('06:00');
    expect(slotOpt.textContent).toContain('06:30');
    expect(slotOpt.textContent).not.toContain('23:00');
    expect(slotOpt.textContent).not.toContain('2026-10-14T23:00:00');
  });

  it('pins slot option label formatting in branch timezone: Asia/Tokyo renders 08:00 and not 06:00 (guards against hardcoded Bangkok)', async () => {
    const BRANCHES = {
      branches: [
        {
          code: 'XC-TYO',
          name: 'สาขาโตเกียว',
          timezone: 'Asia/Tokyo',
          bookingEnabled: true,
        },
      ],
    };
    const SLOTS = {
      data: {
        slots: [
          {
            id: 'slot-tyo-early',
            start: '2026-10-14T23:00:00+00:00',
            end: '2026-10-14T23:30:00+00:00',
            available: 2,
          },
        ],
      },
    };

    const fetchMock = vi.fn(async (url: string) => {
      if (url === '/api/public/host') {
        return new Response(JSON.stringify({ org: 'XC' }), { status: 200 });
      }
      if (url.startsWith('/api/public/orgs/XC/branches')) {
        return new Response(JSON.stringify(BRANCHES), { status: 200 });
      }
      if (url.includes('/booking_slots')) {
        return new Response(JSON.stringify(SLOTS), { status: 200 });
      }
      return new Response('Not found', { status: 404 });
    });

    dispose = render(() => <BookScreen fetch={fetchMock} />, container);

    await vi.waitFor(() => {
      const branchSel = container.querySelector<HTMLSelectElement>('select[name="branchId"]');
      expect(branchSel).not.toBeNull();
      expect(container.querySelector('option[value="XC-TYO"]')).not.toBeNull();
    });

    const branchSel = container.querySelector<HTMLSelectElement>('select[name="branchId"]')!;
    branchSel.value = 'XC-TYO';
    branchSel.dispatchEvent(new Event('change', { bubbles: true }));

    await vi.waitFor(() => {
      const slotOpt = container.querySelector<HTMLOptionElement>('option[value="slot-tyo-early"]');
      expect(slotOpt).not.toBeNull();
      expect(slotOpt!.textContent).toContain('08:00');
    });

    const slotOpt = container.querySelector<HTMLOptionElement>('option[value="slot-tyo-early"]')!;
    // Confirms slot is formatted in chosen branch's timezone (Asia/Tokyo -> 08:00), not hardcoded Bangkok (06:00) or UTC (23:00)
    expect(slotOpt.textContent).toContain('08:00');
    expect(slotOpt.textContent).toContain('08:30');
    expect(slotOpt.textContent).not.toContain('06:00');
    expect(slotOpt.textContent).not.toContain('23:00');
  });
});


