import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import { Dialog, closeDialog } from '../parts/Dialog';
import { Toast } from '../parts/toast';
import { BookingsScreen } from './BookingsScreen';

const DEFAULT_SETTINGS = {
  rules: { enabled: true, minutes: 30 },
  memberCount: 5,
  publicUrl: 'https://example.com/book',
  counterRates: [
    { currency: 'USD', buy: '35.50', sell: '36.00' },
    { currency: 'JPY', buy: '0.24', sell: '0.25' },
  ],
  freshness: { stale: false, sourceDate: '2026-10-14', publishedAt: '10:00' },
};

function makeBookingRow(id: string) {
  return {
    id,
    name: 'Somchai Prasert',
    phone: '0812345678',
    type: 'BUY' as const,
    currency: 'USD',
    amount: '1000',
    rate: '35.50',
    counterRate: '35.50',
    counterPrincipal: '35500',
    status: 'PENDING' as const,
    expiresAt: '2026-10-15T12:00:00Z',
    member: false,
  };
}

function jsonResponse(data: unknown, status = 200) {
  return Promise.resolve(
    new Response(JSON.stringify(data), {
      status,
      headers: { 'content-type': 'application/json' },
    }),
  );
}

function errorResponse(status: number, code: string, message: string) {
  return Promise.resolve(
    new Response(JSON.stringify({ error: { code, message } }), {
      status,
      headers: { 'content-type': 'application/json' },
    }),
  );
}

describe('BookingsScreen settings and branch-name read errors (#2596)', () => {
  let root: HTMLElement;
  let dispose: (() => void) | undefined;

  let bookingsHandler: () => Promise<Response>;
  let settingsHandler: () => Promise<Response>;
  let branchesHandler: () => Promise<Response>;

  beforeEach(() => {
    setLang('th');
    root = document.createElement('div');
    document.body.appendChild(root);

    bookingsHandler = () =>
      jsonResponse({
        version: 1,
        changed: true,
        data: [makeBookingRow('BK-101')],
      });

    settingsHandler = () =>
      jsonResponse({
        version: 1,
        changed: true,
        data: DEFAULT_SETTINGS,
      });

    branchesHandler = () =>
      jsonResponse({
        version: 1,
        changed: true,
        data: { branches: [{ id: 'MAIN', name: 'Main Branch' }] },
      });

    vi.stubGlobal(
      'fetch',
      vi.fn((input: unknown) => {
        const url = String(input);
        if (url.includes('/api/v3/views/admin_bookings')) {
          return bookingsHandler();
        }
        if (url.includes('/api/v3/views/admin_booking_settings')) {
          return settingsHandler();
        }
        if (url.includes('/api/v3/views/admin_branches')) {
          return branchesHandler();
        }
        return jsonResponse({ version: 1, changed: false });
      }),
    );
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    root?.remove();
    closeDialog();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  function mount(): HTMLElement {
    const [branch] = createSignal('MAIN');
    const [period] = createSignal({ from: '', to: '' });
    const [shown] = createSignal(true);
    dispose = render(
      () => (
        <>
          <BookingsScreen branch={branch} period={period} shown={shown} />
          <Dialog />
          <Toast />
        </>
      ),
      root,
    );
    return root;
  }

  function clickRefreshButton() {
    const btn = [...root.querySelectorAll<HTMLButtonElement>('.bookings-actions button')].find(
      (b) => b.textContent?.includes('โหลดการจองล่าสุด'),
    );
    expect(btn, 'refresh bookings button should exist').toBeDefined();
    btn!.click();
  }

  function clickSettingsButton() {
    const btn = [...root.querySelectorAll<HTMLButtonElement>('.bookings-actions button')].find(
      (b) => b.textContent?.includes('ตั้งค่าการจอง'),
    );
    expect(btn, 'booking settings button should exist').toBeDefined();
    btn!.click();
  }

  it('row 1: List OK, settings poll fails -> Table still shown; the settings error is shown', async () => {
    const fiftyRows = Array.from({ length: 50 }, (_, i) => makeBookingRow(`BK-${i + 1}`));
    bookingsHandler = () =>
      jsonResponse({
        version: 1,
        changed: true,
        data: fiftyRows,
      });
    settingsHandler = () => Promise.reject(new TypeError('Failed to fetch'));

    mount();

    await vi.waitFor(() => {
      expect(root.textContent).toContain(sharedTh['error.network']);
    });
    await vi.waitFor(() => {
      expect(root.querySelectorAll('tbody tr').length).toBe(50);
    });

    expect(root.querySelector('.bookings-top')?.textContent).toContain(sharedTh['error.network']);
    expect(root.querySelector('table')).not.toBeNull();
    expect(root.textContent).toContain('BK-1');
    expect(root.querySelector('.load-more button')).not.toBeNull();
    expect(root.querySelector('td button.primary')).not.toBeNull();
  });

  it('row 2: List OK, Booking settings clicked, branch read fails -> Table still shown; error shown', async () => {
    branchesHandler = () => errorResponse(500, 'serverError', 'Branch read failed');

    mount();

    await vi.waitFor(() => {
      expect(root.querySelectorAll('tbody tr').length).toBe(1);
    });

    clickSettingsButton();

    await vi.waitFor(() => {
      expect(root.textContent).toContain(sharedTh['error.serverError']);
    });

    expect(root.querySelector('table')).not.toBeNull();
    expect(root.querySelectorAll('tbody tr').length).toBe(1);
    expect(root.textContent).toContain('BK-101');
    expect(root.querySelector('td button.primary')).not.toBeNull();
    expect(root.querySelector('.bookings-top')?.textContent).toContain(sharedTh['error.serverError']);
    expect(document.querySelector('dialog')?.open ?? false).toBe(false);
  });

  it('row 3: List read fails -> List error shown, no table (unchanged)', async () => {
    bookingsHandler = () => errorResponse(500, 'serverError', 'Bookings list failed');
    branchesHandler = () => Promise.reject(new TypeError('Failed to fetch'));

    mount();

    await vi.waitFor(() => {
      expect(root.textContent).toContain(sharedTh['error.serverError']);
    });

    expect(root.querySelector('table')).toBeNull();
    expect(root.querySelectorAll('tbody tr').length).toBe(0);
    expect(root.querySelector('.bookings-top')?.textContent).not.toContain(sharedTh['error.serverError']);

    clickSettingsButton();

    await vi.waitFor(() => {
      expect(root.textContent).toContain(sharedTh['error.network']);
    });

    expect(root.textContent).toContain(sharedTh['error.serverError']);
    expect(root.querySelector('.bookings-top')?.textContent).toContain(sharedTh['error.network']);
    expect(root.querySelector('table')).toBeNull();
    expect(root.querySelectorAll('tbody tr').length).toBe(0);
  });

  it('edge case: A later successful settings read clears the settings error', async () => {
    settingsHandler = () => Promise.reject(new TypeError('Failed to fetch'));

    mount();

    await vi.waitFor(() => {
      expect(root.textContent).toContain(sharedTh['error.network']);
    });
    await vi.waitFor(() => {
      expect(root.querySelectorAll('tbody tr').length).toBe(1);
    });

    expect(root.querySelector('.bookings-top')?.textContent).toContain(sharedTh['error.network']);

    settingsHandler = () =>
      jsonResponse({
        version: 2,
        changed: true,
        data: DEFAULT_SETTINGS,
      });

    clickRefreshButton();

    await vi.waitFor(() => {
      expect(root.textContent).not.toContain(sharedTh['error.network']);
    });

    expect(root.querySelector('table')).not.toBeNull();
    expect(root.querySelectorAll('tbody tr').length).toBe(1);
  });
});
