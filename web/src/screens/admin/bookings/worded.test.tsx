import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import { en as sharedEn } from '../../../shared/i18n/en';
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

function makeBookingRow(id: string, overrides: Record<string, unknown> = {}) {
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
    ...overrides,
  };
}

describe('WE-5: screens/admin/bookings/ — wordedError handling', () => {
  let root: HTMLElement;
  let dispose: (() => void) | undefined;

  let bookingsHandler: () => Promise<Response>;
  let settingsHandler: () => Promise<Response>;
  let branchesHandler: () => Promise<Response>;
  let pageHandler: () => Promise<Response>;
  let postHandler: () => Promise<Response>;

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

  beforeEach(() => {
    setLang('th');
    root = document.createElement('div');
    document.body.appendChild(root);

    bookingsHandler = () =>
      jsonResponse({
        version: 1,
        changed: true,
        data: [],
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

    pageHandler = () => jsonResponse([]);
    postHandler = () => jsonResponse({ ok: true });

    vi.stubGlobal(
      'fetch',
      vi.fn((input: unknown) => {
        const url = String(input);
        if (url.includes('/api/v3/views/admin_bookings')) {
          if (url.includes('before=')) {
            return pageHandler();
          }
          return bookingsHandler();
        }
        if (url.includes('/api/v3/views/admin_booking_settings')) {
          return settingsHandler();
        }
        if (url.includes('/api/v3/views/admin_branches')) {
          return branchesHandler();
        }
        if (url.includes('/api/v2/bookings')) {
          return postHandler();
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

  function mount(customBranch?: () => string): HTMLElement {
    const [branch] = createSignal('MAIN');
    const [period] = createSignal({ from: '', to: '' });
    const [shown] = createSignal(true);
    dispose = render(
      () => (
        <>
          <BookingsScreen branch={customBranch ?? branch} period={period} shown={shown} />
          <Dialog />
          <Toast />
        </>
      ),
      root,
    );
    return root;
  }

  // =========================================================================
  // Catch 1: fetchBookings
  // =========================================================================
  describe('Catch 1: fetchBookings', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      bookingsHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      mount();

      await vi.waitFor(() => {
        expect(root.textContent).toContain(sharedTh['error.network']);
      });
      expect(root.textContent).not.toContain('Failed to fetch');

      setLang('en');
      const refreshBtn = root.querySelectorAll<HTMLButtonElement>('.bookings-actions button')[0];
      refreshBtn.click();

      await vi.waitFor(() => {
        expect(root.textContent).toContain(sharedEn['error.network']);
      });
    });

    it('(2) 500 refusal shows error.serverError in TH', async () => {
      bookingsHandler = () => errorResponse(500, 'roleNotAllowedHere', 'Internal Error');
      mount();

      await vi.waitFor(() => {
        expect(root.textContent).toContain(sharedTh['error.serverError']);
      });
      expect(root.textContent).not.toContain('Internal Error');
      expect(root.textContent).not.toContain(sharedTh['error.roleNotAllowedHere']);
    });

    it('(3) refusal with unkeyed code and server message shows that message', async () => {
      bookingsHandler = () => errorResponse(400, 'unkeyedBookingCode', 'Custom booking failure detail');
      mount();

      await vi.waitFor(() => {
        expect(root.textContent).toContain('Custom booking failure detail');
      });
      expect(root.textContent).not.toContain('unkeyedBookingCode');
    });
  });

  // =========================================================================
  // Catch 2: fetchSettings
  // =========================================================================
  describe('Catch 2: fetchSettings', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      settingsHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      mount();

      await vi.waitFor(() => {
        expect(root.textContent).toContain(sharedTh['error.network']);
      });
      expect(root.textContent).not.toContain('Failed to fetch');

      setLang('en');
      const refreshBtn = root.querySelectorAll<HTMLButtonElement>('.bookings-actions button')[0];
      refreshBtn.click();

      await vi.waitFor(() => {
        expect(root.textContent).toContain(sharedEn['error.network']);
      });
    });

    it('(2) 500 refusal shows error.serverError in TH', async () => {
      settingsHandler = () => errorResponse(500, 'roleNotAllowedHere', 'Internal Error');
      mount();

      await vi.waitFor(() => {
        expect(root.textContent).toContain(sharedTh['error.serverError']);
      });
      expect(root.textContent).not.toContain('Internal Error');
      expect(root.textContent).not.toContain(sharedTh['error.roleNotAllowedHere']);
    });

    it('(3) refusal with unkeyed code and server message shows that message', async () => {
      settingsHandler = () => errorResponse(400, 'unkeyedSettingsCode', 'Custom settings failure detail');
      mount();

      await vi.waitFor(() => {
        expect(root.textContent).toContain('Custom settings failure detail');
      });
      expect(root.textContent).not.toContain('unkeyedSettingsCode');
    });
  });

  // =========================================================================
  // Catch 3: handleLoadMore
  // =========================================================================
  describe('Catch 3: handleLoadMore', () => {
    async function setup50RowsAndClickLoadMore() {
      const rows = Array.from({ length: 50 }, (_, i) => makeBookingRow(`BK-${i + 1}`));
      bookingsHandler = () =>
        jsonResponse({
          version: 1,
          changed: true,
          data: rows,
        });
      mount();

      await vi.waitFor(() => {
        const btn = root.querySelector('.load-more button');
        expect(btn).not.toBeNull();
      });

      const btn = root.querySelector<HTMLButtonElement>('.load-more button')!;
      btn.click();
    }

    it('(1) network TypeError shows error.network in TH and EN in toast', async () => {
      pageHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      await setup50RowsAndClickLoadMore();

      const toast = root.querySelector('.toast');
      await vi.waitFor(() => {
        expect(toast?.textContent).toContain(sharedTh['error.network']);
      });
      expect(toast?.textContent).not.toContain('Failed to fetch');

      setLang('en');
      const btn = root.querySelector<HTMLButtonElement>('.load-more button')!;
      btn.click();
      await vi.waitFor(() => {
        expect(toast?.textContent).toContain(sharedEn['error.network']);
      });
    });

    it('(2) 500 refusal shows error.serverError in TH in toast', async () => {
      pageHandler = () => errorResponse(500, 'roleNotAllowedHere', 'Internal page error');
      await setup50RowsAndClickLoadMore();

      const toast = root.querySelector('.toast');
      await vi.waitFor(() => {
        expect(toast?.textContent).toContain(sharedTh['error.serverError']);
      });
      expect(toast?.textContent).not.toContain('Internal page error');
      expect(toast?.textContent).not.toContain(sharedTh['error.roleNotAllowedHere']);
    });

    it('(3) refusal with unkeyed code and server message shows that message in toast', async () => {
      pageHandler = () => errorResponse(400, 'unkeyedPageCode', 'Custom page load refusal');
      await setup50RowsAndClickLoadMore();

      const toast = root.querySelector('.toast');
      await vi.waitFor(() => {
        expect(toast?.textContent).toContain('Custom page load refusal');
      });
      expect(toast?.textContent).not.toContain('unkeyedPageCode');
    });
  });

  // =========================================================================
  // Catch 4: confirmRate in openConfirmDialog
  // =========================================================================
  describe('Catch 4: confirmRate (confirm dialog)', () => {
    async function openConfirmAndSubmit() {
      bookingsHandler = () =>
        jsonResponse({
          version: 1,
          changed: true,
          data: [makeBookingRow('BK-CONFIRM-1', { status: 'PENDING' })],
        });
      mount();

      await vi.waitFor(() => {
        const btn = root.querySelector('button.primary');
        expect(btn).not.toBeNull();
      });

      const confirmBtn = root.querySelector<HTMLButtonElement>('td button.primary')!;
      confirmBtn.click();

      await vi.waitFor(() => {
        const input = root.querySelector('input[type="datetime-local"]');
        expect(input).not.toBeNull();
      });

      const expiryInput = root.querySelector<HTMLInputElement>('input[type="datetime-local"]')!;
      expiryInput.value = '2026-10-15T15:00';
      expiryInput.dispatchEvent(new Event('input', { bubbles: true }));

      const submitBtn = root.querySelector<HTMLButtonElement>('.dialog-actions button.primary')!;
      submitBtn.click();
    }

    it('(1) network TypeError shows error.network in TH and EN in dialog', async () => {
      postHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      await openConfirmAndSubmit();

      await vi.waitFor(() => {
        const errEl = root.querySelector('.dialog-body p.error');
        expect(errEl?.textContent).toContain(sharedTh['error.network']);
      });
      expect(root.textContent).not.toContain('Failed to fetch');

      setLang('en');
      const submitBtn = root.querySelector<HTMLButtonElement>('.dialog-actions button.primary')!;
      submitBtn.click();
      await vi.waitFor(() => {
        const errEl = root.querySelector('.dialog-body p.error');
        expect(errEl?.textContent).toContain(sharedEn['error.network']);
      });
    });

    it('(2) 500 refusal shows error.serverError in TH in dialog', async () => {
      postHandler = () => errorResponse(500, 'roleNotAllowedHere', 'Internal confirm error');
      await openConfirmAndSubmit();

      await vi.waitFor(() => {
        const errEl = root.querySelector('.dialog-body p.error');
        expect(errEl?.textContent).toContain(sharedTh['error.serverError']);
      });
      expect(root.textContent).not.toContain('Internal confirm error');
      expect(root.textContent).not.toContain(sharedTh['error.roleNotAllowedHere']);
    });

    it('(3) refusal with unkeyed code and server message shows that message in dialog', async () => {
      postHandler = () => errorResponse(400, 'unkeyedConfirmCode', 'Custom confirm lock refusal');
      await openConfirmAndSubmit();

      await vi.waitFor(() => {
        const errEl = root.querySelector('.dialog-body p.error');
        expect(errEl?.textContent).toContain('Custom confirm lock refusal');
      });
      expect(root.textContent).not.toContain('unkeyedConfirmCode');
    });
  });

  // =========================================================================
  // Catch 5: cancelBooking in openCancelDialog
  // =========================================================================
  describe('Catch 5: cancelBooking (cancel dialog)', () => {
    async function openCancelAndSubmit() {
      bookingsHandler = () =>
        jsonResponse({
          version: 1,
          changed: true,
          data: [makeBookingRow('BK-CANCEL-1', { status: 'PENDING' })],
        });
      mount();

      await vi.waitFor(() => {
        const btn = root.querySelector('button.text-button');
        expect(btn).not.toBeNull();
      });

      const cancelBtn = root.querySelector<HTMLButtonElement>('td button.text-button')!;
      cancelBtn.click();

      await vi.waitFor(() => {
        const confirmBtn = root.querySelector('.dialog-actions button.danger');
        expect(confirmBtn).not.toBeNull();
      });

      const confirmBtn = root.querySelector<HTMLButtonElement>('.dialog-actions button.danger')!;
      confirmBtn.click();
    }

    it('(1) network TypeError shows error.network in TH and EN in toast', async () => {
      postHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      await openCancelAndSubmit();

      const toast = root.querySelector('.toast');
      await vi.waitFor(() => {
        expect(toast?.textContent).toContain(sharedTh['error.network']);
      });
      expect(toast?.textContent).not.toContain('Failed to fetch');

      setLang('en');
      // Click cancel again
      const cancelBtn = root.querySelector<HTMLButtonElement>('td button.text-button')!;
      cancelBtn.click();
      await vi.waitFor(() => {
        expect(root.querySelector('.dialog-actions button.danger')).not.toBeNull();
      });
      root.querySelector<HTMLButtonElement>('.dialog-actions button.danger')!.click();

      await vi.waitFor(() => {
        expect(toast?.textContent).toContain(sharedEn['error.network']);
      });
    });

    it('(2) 500 refusal shows error.serverError in TH in toast', async () => {
      postHandler = () => errorResponse(500, 'roleNotAllowedHere', 'Internal cancel error');
      await openCancelAndSubmit();

      const toast = root.querySelector('.toast');
      await vi.waitFor(() => {
        expect(toast?.textContent).toContain(sharedTh['error.serverError']);
      });
      expect(toast?.textContent).not.toContain('Internal cancel error');
      expect(toast?.textContent).not.toContain(sharedTh['error.roleNotAllowedHere']);
    });

    it('(3) refusal with unkeyed code and server message shows that message in toast', async () => {
      postHandler = () => errorResponse(400, 'unkeyedCancelCode', 'Custom cancel refusal message');
      await openCancelAndSubmit();

      const toast = root.querySelector('.toast');
      await vi.waitFor(() => {
        expect(toast?.textContent).toContain('Custom cancel refusal message');
      });
      expect(toast?.textContent).not.toContain('unkeyedCancelCode');
    });
  });

  // =========================================================================
  // Catch 6: openSettingsDialog branch view failure
  // =========================================================================
  describe('Catch 6: openSettingsDialog branch view', () => {
    async function clickSettingsButton() {
      mount();
      await vi.waitFor(() => {
        const btns = [...root.querySelectorAll('.bookings-actions button')];
        expect(btns.length).toBeGreaterThan(1);
      });
      // The 2nd button in bookings-actions is settings ("bookingSettings")
      const btns = [...root.querySelectorAll<HTMLButtonElement>('.bookings-actions button')];
      btns[1].click();
    }

    it('(1) network TypeError shows error.network in TH and EN', async () => {
      branchesHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      await clickSettingsButton();

      await vi.waitFor(() => {
        expect(root.textContent).toContain(sharedTh['error.network']);
      });
      expect(root.textContent).not.toContain('Failed to fetch');

      setLang('en');
      const btns = [...root.querySelectorAll<HTMLButtonElement>('.bookings-actions button')];
      btns[1].click();
      await vi.waitFor(() => {
        expect(root.textContent).toContain(sharedEn['error.network']);
      });
    });

    it('(2) 500 refusal shows error.serverError in TH', async () => {
      branchesHandler = () => errorResponse(500, 'roleNotAllowedHere', 'Internal branch error');
      await clickSettingsButton();

      await vi.waitFor(() => {
        expect(root.textContent).toContain(sharedTh['error.serverError']);
      });
      expect(root.textContent).not.toContain('Internal branch error');
      expect(root.textContent).not.toContain(sharedTh['error.roleNotAllowedHere']);
    });

    it('(3) refusal with unkeyed code and server message shows that message', async () => {
      branchesHandler = () => errorResponse(400, 'unkeyedBranchCode', 'Custom branches fetch refusal');
      await clickSettingsButton();

      await vi.waitFor(() => {
        expect(root.textContent).toContain('Custom branches fetch refusal');
      });
      expect(root.textContent).not.toContain('unkeyedBranchCode');
    });
  });

  // =========================================================================
  // Catch 7: saveSettings in openSettingsDialog
  // =========================================================================
  describe('Catch 7: saveSettings in openSettingsDialog', () => {
    async function openSettingsAndSave() {
      mount();
      await vi.waitFor(() => {
        const btns = [...root.querySelectorAll<HTMLButtonElement>('.bookings-actions button')];
        expect(btns.length).toBeGreaterThan(1);
      });
      const btns = [...root.querySelectorAll<HTMLButtonElement>('.bookings-actions button')];
      btns[1].click();

      await vi.waitFor(() => {
        const saveBtn = root.querySelector('.dialog-body button.primary');
        expect(saveBtn).not.toBeNull();
      });

      const saveBtn = root.querySelector<HTMLButtonElement>('.dialog-body button.primary')!;
      saveBtn.click();
    }

    it('(1) network TypeError shows error.network in TH and EN in dialog', async () => {
      postHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      await openSettingsAndSave();

      await vi.waitFor(() => {
        const errEl = root.querySelector('.dialog-body p.error');
        expect(errEl?.textContent).toContain(sharedTh['error.network']);
      });
      expect(root.textContent).not.toContain('Failed to fetch');

      setLang('en');
      const saveBtn = root.querySelector<HTMLButtonElement>('.dialog-body button.primary')!;
      saveBtn.click();
      await vi.waitFor(() => {
        const errEl = root.querySelector('.dialog-body p.error');
        expect(errEl?.textContent).toContain(sharedEn['error.network']);
      });
    });

    it('(2) 500 refusal shows error.serverError in TH in dialog', async () => {
      postHandler = () => errorResponse(500, 'roleNotAllowedHere', 'Internal settings save error');
      await openSettingsAndSave();

      await vi.waitFor(() => {
        const errEl = root.querySelector('.dialog-body p.error');
        expect(errEl?.textContent).toContain(sharedTh['error.serverError']);
      });
      expect(root.textContent).not.toContain('Internal settings save error');
      expect(root.textContent).not.toContain(sharedTh['error.roleNotAllowedHere']);
    });

    it('(3) refusal with unkeyed code and server message shows that message in dialog', async () => {
      postHandler = () => errorResponse(400, 'unkeyedSettingsSaveCode', 'Custom settings save refusal');
      await openSettingsAndSave();

      await vi.waitFor(() => {
        const errEl = root.querySelector('.dialog-body p.error');
        expect(errEl?.textContent).toContain('Custom settings save refusal');
      });
      expect(root.textContent).not.toContain('unkeyedSettingsSaveCode');
    });
  });

  // =========================================================================
  // Catch 8: handleCreateSubmit in openCreateDialog
  // =========================================================================
  describe('Catch 8: handleCreateSubmit in openCreateDialog', () => {
    async function openCreateAndSubmit() {
      mount();
      await vi.waitFor(() => {
        expect(root.querySelector('.info-line')?.textContent).toContain('5');
      });
      // The 3rd button is createButtonWithPlus
      const btns = [...root.querySelectorAll<HTMLButtonElement>('.bookings-actions button')];
      btns[2].click();

      await vi.waitFor(() => {
        const form = root.querySelector('form');
        expect(form).not.toBeNull();
      });

      // Fill valid inputs
      const textInputs = [...root.querySelectorAll<HTMLInputElement>('form input[type="text"]')];
      const telInput = root.querySelector<HTMLInputElement>('form input[type="tel"]')!;

      // textInputs: [name, amount, rate, customerId]
      textInputs[0].value = 'Customer Name';
      textInputs[0].dispatchEvent(new Event('input', { bubbles: true }));

      telInput.value = '0812345678';
      telInput.dispatchEvent(new Event('input', { bubbles: true }));

      textInputs[1].value = '500';
      textInputs[1].dispatchEvent(new Event('input', { bubbles: true }));

      const submitBtn = root.querySelector<HTMLButtonElement>('form button[type="submit"]')!;
      submitBtn.click();
    }

    it('(1) network TypeError shows error.network in TH and EN in dialog', async () => {
      postHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      await openCreateAndSubmit();

      await vi.waitFor(() => {
        const errEl = root.querySelector('form p.error');
        expect(errEl?.textContent).toContain(sharedTh['error.network']);
      });
      expect(root.textContent).not.toContain('Failed to fetch');

      setLang('en');
      const submitBtn = root.querySelector<HTMLButtonElement>('form button[type="submit"]')!;
      submitBtn.click();
      await vi.waitFor(() => {
        const errEl = root.querySelector('form p.error');
        expect(errEl?.textContent).toContain(sharedEn['error.network']);
      });
    });

    it('(2) 500 refusal shows error.serverError in TH in dialog', async () => {
      postHandler = () => errorResponse(500, 'roleNotAllowedHere', 'Internal create error');
      await openCreateAndSubmit();

      await vi.waitFor(() => {
        const errEl = root.querySelector('form p.error');
        expect(errEl?.textContent).toContain(sharedTh['error.serverError']);
      });
      expect(root.textContent).not.toContain('Internal create error');
      expect(root.textContent).not.toContain(sharedTh['error.roleNotAllowedHere']);
    });

    it('(3) refusal with unkeyed code and server message shows that message in dialog', async () => {
      postHandler = () => errorResponse(400, 'unkeyedCreateCode', 'Custom booking create refusal');
      await openCreateAndSubmit();

      await vi.waitFor(() => {
        const errEl = root.querySelector('form p.error');
        expect(errEl?.textContent).toContain('Custom booking create refusal');
      });
      expect(root.textContent).not.toContain('unkeyedCreateCode');
    });
  });

  // =========================================================================
  // Criterion 3: Stale data cleared on branch switch
  // =========================================================================
  describe('Criterion 3: Branch switch clears stale data', () => {
    it('after branch switch, a failed read does not leave the previous branch data showing', async () => {
      const [currentBranch, setCurrentBranch] = createSignal('MAIN');

      bookingsHandler = () => {
        const b = currentBranch();
        if (b === 'MAIN') {
          return jsonResponse({
            version: 1,
            changed: true,
            data: [makeBookingRow('BK-MAIN-STALE-1', { name: 'Main Branch Customer' })],
          });
        }
        return errorResponse(500, 'roleNotAllowedHere', 'Branch 2 unavailable');
      };

      mount(currentBranch);

      await vi.waitFor(() => {
        expect(root.textContent).toContain('BK-MAIN-STALE-1');
        expect(root.textContent).toContain('Main Branch Customer');
      });

      // Switch branch to BRANCH-2 which fails
      setCurrentBranch('BRANCH-2');

      await vi.waitFor(() => {
        expect(root.textContent).toContain(sharedTh['error.serverError']);
      });

      // Stale data from MAIN branch MUST NOT be showing
      expect(root.textContent).not.toContain('BK-MAIN-STALE-1');
      expect(root.textContent).not.toContain('Main Branch Customer');
    });
  });
});
