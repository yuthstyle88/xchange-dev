// #2553: Admin Bookings Create dialog derives chosen() currency from availableCurrencies()
// when opened before settings load or across a branch switch while open.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { Dialog, closeDialog } from '../parts/Dialog';
import { Toast } from '../parts/toast';
import { BookingsScreen } from './BookingsScreen';
import { th } from './i18n';

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

describe('108-Plaza/xchange#2553: Admin Bookings Create currency derivation (§8, §9)', () => {
  let root: HTMLElement;
  let dispose: (() => void) | undefined;
  let setBranchSignal!: (b: string) => void;
  let settingsResponder: (branch: string) => Promise<Response>;
  let createPayloads: Array<Record<string, unknown>>;

  beforeEach(() => {
    setLang('th');
    createPayloads = [];

    settingsResponder = () =>
      Promise.resolve(
        new Response(
          JSON.stringify({
            version: 1,
            changed: true,
            data: {
              rules: { enabled: true, minutes: 30 },
              memberCount: 5,
              publicUrl: 'https://example.com/book',
              counterRates: [
                { currency: 'USD', buy: '35.50', sell: '36.00' },
                { currency: 'EUR', buy: '38.10', sell: '38.80' },
              ],
              freshness: { stale: false, sourceDate: '2026-10-16', publishedAt: '2026-10-16T03:00:00Z' },
            },
          }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        ),
      );

    vi.stubGlobal(
      'fetch',
      vi.fn((input: unknown, init?: RequestInit) => {
        const url = new URL(String(input), 'http://localhost');
        const method = (init?.method ?? 'GET').toUpperCase();

        if (url.pathname === '/api/v3/views/admin_booking_settings' && method === 'GET') {
          const branch = url.searchParams.get('branch') ?? 'MAIN';
          return settingsResponder(branch);
        }

        if (url.pathname === '/api/v3/views/admin_bookings' && method === 'GET') {
          return Promise.resolve(
            new Response(
              JSON.stringify({ version: 1, changed: true, data: [] }),
              { status: 200, headers: { 'content-type': 'application/json' } },
            ),
          );
        }

        if (url.pathname === '/api/v2/bookings' && method === 'POST') {
          const parsed = JSON.parse(String(init?.body ?? '{}')) as {
            action?: string;
            payload?: Record<string, unknown>;
          };
          if (parsed.action === 'create' && parsed.payload) {
            createPayloads.push(parsed.payload);
          }
          return Promise.resolve(
            new Response(JSON.stringify({ ok: true }), {
              status: 200,
              headers: { 'content-type': 'application/json' },
            }),
          );
        }

        return Promise.resolve(
          new Response(JSON.stringify({ version: 1, changed: false }), {
            status: 200,
            headers: { 'content-type': 'application/json' },
          }),
        );
      }),
    );

    root = document.createElement('div');
    document.body.appendChild(root);
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

  function mount(initialBranch = 'MAIN'): HTMLElement {
    const [branch, setBranch] = createSignal(initialBranch);
    setBranchSignal = setBranch;
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

  function getCreateButton(): HTMLButtonElement {
    const buttons = [...root.querySelectorAll('button')];
    const btn = buttons.find(
      (b) => b.textContent?.includes('สร้างการจอง') || b.textContent?.includes('Create booking'),
    );
    expect(btn, 'create booking button should exist').toBeDefined();
    return btn!;
  }

  function getCurrencySelect(dialog: HTMLElement): HTMLSelectElement {
    const labels = [...dialog.querySelectorAll('label')];
    const currencyLabel = labels.find(
      (l) => l.textContent?.includes('สกุลเงิน') || l.textContent?.includes('Currency'),
    );
    expect(currencyLabel, 'currency label should exist').toBeDefined();
    const select = currencyLabel!.querySelector('select');
    expect(select, 'currency select should exist').toBeDefined();
    return select!;
  }

  function getRateInput(dialog: HTMLElement): HTMLInputElement {
    const input = dialog.querySelector<HTMLInputElement>('input[readonly]');
    expect(input, 'readonly rate input should exist').not.toBeNull();
    return input!;
  }

  function fillNamePhoneAmount(dialog: HTMLElement, name: string, phone: string, amount: string) {
    const textInputs = [...dialog.querySelectorAll<HTMLInputElement>('input[type="text"]:not([readonly])')];
    const nameInput = textInputs[0]!;
    const amountInput = textInputs[1]!;
    const phoneInput = dialog.querySelector<HTMLInputElement>('input[type="tel"]')!;

    nameInput.value = name;
    nameInput.dispatchEvent(new Event('input', { bubbles: true }));
    phoneInput.value = phone;
    phoneInput.dispatchEvent(new Event('input', { bubbles: true }));
    amountInput.value = amount;
    amountInput.dispatchEvent(new Event('input', { bubbles: true }));
  }

  it('Row 1: Open Create with settings null, settings then arrive with EUR, USD -> Select shows EUR, rate filled, Create sends EUR', async () => {
    const gate = deferred<Response>();
    settingsResponder = () => gate.promise;

    mount();
    await vi.waitFor(() => {
      expect(root.querySelector('.admin-bookings')).not.toBeNull();
    });

    // Open Create while settings() is still null
    getCreateButton().click();

    await vi.waitFor(() => {
      expect(document.querySelector('dialog')).not.toBeNull();
    });
    const dialog = document.querySelector('dialog')!;
    const select = getCurrencySelect(dialog);
    const rateInput = getRateInput(dialog);

    expect(select.value).toBe('');
    expect(rateInput.value).toBe('');

    // Settings now arrive with USD and EUR (sorted availableCurrencies: ['EUR', 'USD'])
    gate.resolve(
      new Response(
        JSON.stringify({
          version: 1,
          changed: true,
          data: {
            rules: { enabled: true, minutes: 30 },
            memberCount: 5,
            publicUrl: 'https://example.com/book',
            counterRates: [
              { currency: 'USD', buy: '35.50', sell: '36.00' },
              { currency: 'EUR', buy: '38.10', sell: '38.80' },
            ],
            freshness: { stale: false, sourceDate: '2026-10-16', publishedAt: '2026-10-16T03:00:00Z' },
          },
        }),
        { status: 200, headers: { 'content-type': 'application/json' } },
      ),
    );

    await vi.waitFor(() => {
      expect(select.value).toBe('EUR');
      expect(rateInput.value).toBe('38.10');
    });

    fillNamePhoneAmount(dialog, 'Somchai', '0812345678', '100');
    dialog.querySelector<HTMLButtonElement>('button[type="submit"]')!.click();

    await vi.waitFor(() => {
      expect(createPayloads.length).toBe(1);
    });
    expect(dialog.querySelector('.error')).toBeNull();
    expect(createPayloads[0]).toMatchObject({
      branchId: 'MAIN',
      name: 'Somchai',
      phone: '0812345678',
      type: 'BUY',
      currency: 'EUR',
      amount: '100',
      expectedRate: '38.10',
    });
  });

  it('Row 2: User picks USD afterwards -> USD used for select, rate and Create payload', async () => {
    const gate = deferred<Response>();
    settingsResponder = () => gate.promise;

    mount();
    await vi.waitFor(() => {
      expect(root.querySelector('.admin-bookings')).not.toBeNull();
    });

    getCreateButton().click();
    await vi.waitFor(() => {
      expect(document.querySelector('dialog')).not.toBeNull();
    });
    const dialog = document.querySelector('dialog')!;
    const select = getCurrencySelect(dialog);
    const rateInput = getRateInput(dialog);

    gate.resolve(
      new Response(
        JSON.stringify({
          version: 1,
          changed: true,
          data: {
            rules: { enabled: true, minutes: 30 },
            memberCount: 5,
            publicUrl: 'https://example.com/book',
            counterRates: [
              { currency: 'USD', buy: '35.50', sell: '36.00' },
              { currency: 'EUR', buy: '38.10', sell: '38.80' },
            ],
            freshness: { stale: false, sourceDate: '2026-10-16', publishedAt: '2026-10-16T03:00:00Z' },
          },
        }),
        { status: 200, headers: { 'content-type': 'application/json' } },
      ),
    );

    await vi.waitFor(() => {
      expect(select.value).toBe('EUR');
      expect(rateInput.value).toBe('38.10');
    });

    // User picks USD afterwards
    select.value = 'USD';
    select.dispatchEvent(new Event('change', { bubbles: true }));

    await vi.waitFor(() => {
      expect(select.value).toBe('USD');
      expect(rateInput.value).toBe('35.50');
    });

    fillNamePhoneAmount(dialog, 'Suda', '0898765432', '250');
    dialog.querySelector<HTMLButtonElement>('button[type="submit"]')!.click();

    await vi.waitFor(() => {
      expect(createPayloads.length).toBe(1);
    });
    expect(createPayloads[0]).toMatchObject({
      branchId: 'MAIN',
      name: 'Suda',
      phone: '0898765432',
      type: 'BUY',
      currency: 'USD',
      amount: '250',
      expectedRate: '35.50',
    });
  });

  it('Row 3: Settings stay empty -> select.value === "", noCurrencies shown, no POST; when rates later arrive on refresh, chosen() updates', async () => {
    let counterRates: Array<{ currency: string; buy: string; sell: string }> = [];
    settingsResponder = () =>
      Promise.resolve(
        new Response(
          JSON.stringify({
            version: 1,
            changed: true,
            data: {
              rules: { enabled: true, minutes: 45 },
              memberCount: 0,
              publicUrl: '',
              counterRates,
              freshness: { stale: false, sourceDate: null, publishedAt: null },
            },
          }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        ),
      );

    mount();
    await vi.waitFor(() => {
      expect(root.textContent).toContain('45');
    });

    getCreateButton().click();
    await vi.waitFor(() => {
      expect(document.querySelector('dialog')).not.toBeNull();
    });
    const dialog = document.querySelector('dialog')!;
    const select = getCurrencySelect(dialog);
    const rateInput = getRateInput(dialog);

    expect(select.value).toBe('');
    expect(rateInput.value).toBe('');
    expect(dialog.textContent).toContain(th.noCurrencies);

    fillNamePhoneAmount(dialog, 'Somchai', '0812345678', '100');
    dialog.querySelector<HTMLButtonElement>('button[type="submit"]')!.click();

    await vi.waitFor(() => {
      expect(dialog.querySelector('.error')?.textContent).toBe(th.noCurrencies);
    });
    expect(createPayloads).toHaveLength(0);

    // If settings later populate with EUR, USD on refresh while dialog is open, chosen() picks EUR
    counterRates = [
      { currency: 'USD', buy: '35.50', sell: '36.00' },
      { currency: 'EUR', buy: '38.10', sell: '38.80' },
    ];
    const refreshBtn = [...root.querySelectorAll('button')].find(
      (b) => b.textContent?.includes('โหลดการจองล่าสุด') || b.textContent?.includes('Reload latest bookings'),
    )!;
    refreshBtn.click();

    await vi.waitFor(() => {
      expect(select.value).toBe('EUR');
      expect(rateInput.value).toBe('38.10');
    });
  });

  it('Edge case: Branch switch while the dialog is open -> chosen() follows the new branch currency list', async () => {
    settingsResponder = (branch) => {
      const counterRates =
        branch === 'B2'
          ? [
              { currency: 'JPY', buy: '0.24', sell: '0.25' },
              { currency: 'CHF', buy: '39.00', sell: '40.00' },
            ]
          : [
              { currency: 'GBP', buy: '44.00', sell: '45.00' },
              { currency: 'USD', buy: '35.50', sell: '36.00' },
            ];
      return Promise.resolve(
        new Response(
          JSON.stringify({
            version: 1,
            changed: true,
            data: {
              rules: { enabled: true, minutes: 45 },
              memberCount: 5,
              publicUrl: 'https://example.com/book',
              counterRates,
              freshness: { stale: false, sourceDate: '2026-10-16', publishedAt: '2026-10-16T03:00:00Z' },
            },
          }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        ),
      );
    };

    mount('MAIN');
    // Wait until MAIN settings are loaded before opening Create
    await vi.waitFor(() => {
      expect(root.textContent).toContain('45');
    });

    getCreateButton().click();
    await vi.waitFor(() => {
      expect(document.querySelector('dialog')).not.toBeNull();
    });
    const dialog = document.querySelector('dialog')!;
    const select = getCurrencySelect(dialog);
    const rateInput = getRateInput(dialog);

    expect(select.value).toBe('GBP');
    expect(rateInput.value).toBe('44.00');

    // Switch branch to B2 while the Create dialog remains open (GBP is not in B2's ['CHF', 'JPY'])
    setBranchSignal('B2');

    await vi.waitFor(() => {
      expect(select.value).toBe('CHF');
      expect(rateInput.value).toBe('39.00');
    });

    fillNamePhoneAmount(dialog, 'Arthit', '0823456789', '500');
    dialog.querySelector<HTMLButtonElement>('button[type="submit"]')!.click();

    await vi.waitFor(() => {
      expect(createPayloads.length).toBe(1);
    });
    expect(createPayloads[0]).toMatchObject({
      branchId: 'B2',
      name: 'Arthit',
      phone: '0823456789',
      type: 'BUY',
      currency: 'CHF',
      amount: '500',
      expectedRate: '39.00',
    });
  });
});
