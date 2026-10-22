// Tests for SF-4 / B4: /app/book slots read error and parse handling (#4694, web-silent-failures.md).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import { BookScreen } from './Book';

describe('SF-4 / B4: /app/book slots read error and parse handling (#4694)', () => {
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
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  const SAMPLE_BRANCHES = {
    branches: [
      {
        id: 'XC-B01',
        code: 'XC-B01',
        name: 'สาขาสยามพารากอน',
        timezone: 'Asia/Bangkok',
        bookingEnabled: true,
      },
    ],
  };

  const SAMPLE_SLOTS = {
    version: 1,
    changed: true,
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

  it('K1: the slots read answers 200 with <… shows error.loadFailed with retry', async () => {
    const fetchMock = vi.fn(async (url: string) => {
      if (url === '/api/public/host') {
        return new Response(JSON.stringify({ org: 'XC' }), { status: 200 });
      }
      if (url.includes('/branches')) {
        return new Response(JSON.stringify(SAMPLE_BRANCHES), { status: 200 });
      }
      if (url.includes('/booking_slots')) {
        return new Response('<!DOCTYPE html><html><body>Error fallback page</body></html>', {
          status: 200,
          headers: { 'Content-Type': 'text/html' },
        });
      }
      return new Response('Not found', { status: 404 });
    });

    dispose = render(() => <BookScreen fetch={fetchMock} />, container);

    await vi.waitFor(() => {
      expect(container.querySelector('option[value="XC-B01"]')).not.toBeNull();
    });

    const branchSel = container.querySelector<HTMLSelectElement>('select[name="branchId"]')!;
    branchSel.value = 'XC-B01';
    branchSel.dispatchEvent(new Event('change', { bubbles: true }));

    await vi.waitFor(() => {
      const errorEl = container.querySelector('.xc-book-slots-error');
      expect(errorEl).not.toBeNull();
      expect(errorEl?.textContent).toContain('โหลดข้อมูลไม่สำเร็จ');
      const retryBtn = container.querySelector('.xc-book-slots-error button');
      expect(retryBtn).not.toBeNull();
    });

    expect(container.querySelector('.xc-book-slots-empty')).toBeNull();
  });

  it('K2: the slots read answers 500 shows error.loadFailed with retry (as K1)', async () => {
    const fetchMock = vi.fn(async (url: string) => {
      if (url === '/api/public/host') {
        return new Response(JSON.stringify({ org: 'XC' }), { status: 200 });
      }
      if (url.includes('/branches')) {
        return new Response(JSON.stringify(SAMPLE_BRANCHES), { status: 200 });
      }
      if (url.includes('/booking_slots')) {
        return new Response(JSON.stringify({ error: 'internal_error' }), { status: 500 });
      }
      return new Response('Not found', { status: 404 });
    });

    dispose = render(() => <BookScreen fetch={fetchMock} />, container);

    await vi.waitFor(() => {
      expect(container.querySelector('option[value="XC-B01"]')).not.toBeNull();
    });

    const branchSel = container.querySelector<HTMLSelectElement>('select[name="branchId"]')!;
    branchSel.value = 'XC-B01';
    branchSel.dispatchEvent(new Event('change', { bubbles: true }));

    await vi.waitFor(() => {
      const errorEl = container.querySelector('.xc-book-slots-error');
      expect(errorEl).not.toBeNull();
      expect(errorEl?.textContent).toContain('โหลดข้อมูลไม่สำเร็จ');
      const retryBtn = container.querySelector('.xc-book-slots-error button');
      expect(retryBtn).not.toBeNull();
    });

    expect(container.querySelector('.xc-book-slots-empty')).toBeNull();
  });

  it('K3: the slots read answers 200 with an empty slot list shows "no slots", as today', async () => {
    const fetchMock = vi.fn(async (url: string) => {
      if (url === '/api/public/host') {
        return new Response(JSON.stringify({ org: 'XC' }), { status: 200 });
      }
      if (url.includes('/branches')) {
        return new Response(JSON.stringify(SAMPLE_BRANCHES), { status: 200 });
      }
      if (url.includes('/booking_slots')) {
        return new Response(JSON.stringify({ version: 1, changed: false, data: { slots: [] } }), { status: 200 });
      }
      return new Response('Not found', { status: 404 });
    });

    dispose = render(() => <BookScreen fetch={fetchMock} />, container);

    await vi.waitFor(() => {
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

    expect(container.querySelector('.xc-book-slots-error')).toBeNull();
  });

  it('K4: after K1, retry, and the read now answers valid slots, the slots render', async () => {
    let slotCalls = 0;
    const fetchMock = vi.fn(async (url: string) => {
      if (url === '/api/public/host') {
        return new Response(JSON.stringify({ org: 'XC' }), { status: 200 });
      }
      if (url.includes('/branches')) {
        return new Response(JSON.stringify(SAMPLE_BRANCHES), { status: 200 });
      }
      if (url.includes('/booking_slots')) {
        slotCalls++;
        if (slotCalls === 1) {
          return new Response('<!DOCTYPE html><html><body>Error fallback page</body></html>', {
            status: 200,
            headers: { 'Content-Type': 'text/html' },
          });
        }
        return new Response(JSON.stringify(SAMPLE_SLOTS), { status: 200 });
      }
      return new Response('Not found', { status: 404 });
    });

    dispose = render(() => <BookScreen fetch={fetchMock} />, container);

    await vi.waitFor(() => {
      expect(container.querySelector('option[value="XC-B01"]')).not.toBeNull();
    });

    const branchSel = container.querySelector<HTMLSelectElement>('select[name="branchId"]')!;
    branchSel.value = 'XC-B01';
    branchSel.dispatchEvent(new Event('change', { bubbles: true }));

    await vi.waitFor(() => {
      expect(slotCalls).toBeGreaterThanOrEqual(1);
      const errorEl = container.querySelector('.xc-book-slots-error');
      expect(errorEl).not.toBeNull();
    });

    const retryBtn = container.querySelector<HTMLButtonElement>('.xc-book-slots-error button');
    expect(retryBtn).not.toBeNull();
    retryBtn!.click();

    await vi.waitFor(() => {
      expect(slotCalls).toBeGreaterThanOrEqual(2);
      expect(container.querySelector('.xc-book-slots-error')).toBeNull();
      const slotOpt = container.querySelector<HTMLOptionElement>('select[name="slotId"] option[value="slot-101"]');
      expect(slotOpt).not.toBeNull();
    });

    expect(container.querySelector('.xc-book-slots-empty')).toBeNull();
  });
});
