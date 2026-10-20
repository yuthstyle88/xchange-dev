import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiClient, ApiError } from '../../shared/api/client';
import { makeQuoteDto, makeQuoteLineDto, makeQuotePublicationDto } from '../../shared/testing/fixtures';
import { setLang } from '../../shared/i18n';
import { en as sharedEn } from '../../shared/i18n/en';
import { th as sharedTh } from '../../shared/i18n/th';
import { wordedError } from '../../shared/i18n/worded-error';
import { OWNER_DESK } from './desk-path';
import { createDeskStore, type DeskStore } from './desk-store';
import { step as rateStep } from './steps/rate/step';
import { step as receiptStep } from './steps/receipt/step';

const ME = () => ({
  userId: 'u-1',
  username: 'ploy',
  name: 'Ploy',
  roles: [],
  branch: 'MAIN',
  branchId: 'MAIN',
  branchName: 'XC-B',
  branches: [],
});

const PASSPORT = new File([new Uint8Array([1, 2, 3])], 'passport.jpg', { type: 'image/jpeg' });

type Answer = { status: number; body?: unknown; raw?: string };
type Route = () => Answer | Promise<Answer>;

function fakeServer() {
  const routes = new Map<string, Route>();
  const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const method = String(init?.method ?? 'GET').toUpperCase();
    const path = String(input).split('?')[0]!;
    const route = routes.get(`${method} ${path}`);
    const a: Answer = route
      ? await route()
      : { status: 404, body: { error: { code: 'notFound', message: 'not found' } } };
    const text = a.raw ?? JSON.stringify(a.body ?? {});
    return new Response(text, {
      status: a.status,
      headers: { 'content-type': a.raw ? 'text/html' : 'application/json' },
    });
  });

  return {
    api: new ApiClient(fetchFn as unknown as typeof fetch),
    on(method: string, path: string, route: Route) {
      routes.set(`${method.toUpperCase()} ${path}`, route);
    },
  };
}

function quoteDto(quantity = '100') {
  return makeQuoteDto({
    id: 'q-1',
    type: 'BUY',
    channel: 'DESK',
    expiresAt: new Date(Date.now() + 300_000).toISOString(),
    publication: makeQuotePublicationDto({ id: 'pub-1', version: 3 }),
    lines: [
      makeQuoteLineDto({
        currency: 'USD',
        shopSide: 'BUY',
        quantity,
        unitSize: '1',
        baseRate: '32.10',
        finalRate: '32.10',
        thbAmount: '3210.00',
        feeThb: '0.00',
        settleThb: '3210.00',
        publishedRateLineId: 'line-1',
      }),
    ],
    benefits: [],
    stale: false,
  });
}

describe('Desk worded errors (#2157)', () => {
  let disposers: (() => void)[] = [];
  let host: HTMLDivElement;

  beforeEach(() => {
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    for (const d of disposers) d();
    disposers = [];
    host.remove();
    setLang('th');
  });

  function makeStore(s: ReturnType<typeof fakeServer>): DeskStore {
    s.on('GET', '/api/v3/views/desk_config', () => ({
      status: 200,
      body: {
        version: 1,
        changed: true,
        data: { shiftOpen: true, rates: [{ code: 'USD', buy: 32.1, sell: 32.5 }] },
      },
    }));
    let store!: DeskStore;
    createRoot((dispose) => {
      disposers.push(dispose);
      store = createDeskStore({ api: s.api, path: OWNER_DESK, me: ME as any });
    });
    return store;
  }

  async function readyForSave(s: ReturnType<typeof fakeServer>): Promise<DeskStore> {
    s.on('POST', '/api/quotes', () => ({ status: 200, body: quoteDto() }));
    s.on('POST', '/api/upload', () => ({ status: 200, body: { uploadKey: 'key-1' } }));
    const store = makeStore(s);
    await vi.waitFor(() => expect(store.config()).toBeDefined());
    store.setType('BUY');
    store.setCurrency('USD');
    store.setAmount('100');
    await store.requote();
    store.setPassport(PASSPORT);
    store.setField('name', 'JOHN DOE', 'user');
    store.setField('passportNo', 'AA1234567', 'user');
    store.setField('nationality', 'USA', 'user');
    store.setField('birthDate', '1990-01-01', 'user');
    store.setField('passportExpiry', '2030-01-01', 'user');
    store.setIdentityChecked(true);
    store.setCustomerId('CX-00000001');
    store.setValueDate('2026-10-11');
    store.setMethod('Cash');
    store.setAccount('Cash drawer 1');
    store.setBankFee('0');
    store.setNotes('');
    store.setPaymentChecked(true);
    return store;
  }

  it('row 1: save refused 403 roleNotAllowedHere with message "Invalid organization ID" in TH shows the TH worded text, not the English message', async () => {
    setLang('th');
    const s = fakeServer();
    s.on('POST', '/api/transactions', () => ({
      status: 403,
      body: { error: { code: 'roleNotAllowedHere', message: 'Invalid organization ID' } },
    }));
    const store = await readyForSave(s);
    const disposeView = render(() => receiptStep.component({ store }), host);
    disposers.push(disposeView);

    await expect(store.submit()).resolves.toBe('refused');
    expect(store.saveError()).toBe(sharedTh['error.roleNotAllowedHere']);
    expect(store.saveError()).not.toContain('Invalid organization ID');
    expect(host.querySelector('[role="alert"]')?.textContent).toBe(sharedTh['error.roleNotAllowedHere']);
    expect(host.textContent).not.toContain('Invalid organization ID');
  });

  it('row 2: quote fails 502 with an HTML body shows error.serverError text and no HTML in any language', async () => {
    const htmlBody = '<html><body><h1>502 Bad Gateway</h1><p>nginx proxy error</p></body></html>';
    const s = fakeServer();
    s.on('POST', '/api/quotes', () => ({
      status: 502,
      raw: htmlBody,
    }));
    const store = makeStore(s);
    await vi.waitFor(() => expect(store.config()).toBeDefined());
    const disposeView = render(() => rateStep.component({ store }), host);
    disposers.push(disposeView);

    setLang('th');
    store.setCurrency('USD');
    store.setAmount('100');
    await store.requote();
    expect(store.quoteError()).toEqual({ message: sharedTh['error.serverError'] });
    expect(host.querySelector('.desk-error')?.textContent).toBe(sharedTh['error.serverError']);
    expect(host.textContent).not.toContain('<html>');
    expect(host.textContent).not.toContain('Bad Gateway');

    setLang('en');
    await store.requote();
    expect(store.quoteError()).toEqual({ message: sharedEn['error.serverError'] });
    expect(host.querySelector('.desk-error')?.textContent).toBe(sharedEn['error.serverError']);
    expect(host.textContent).not.toContain('<html>');
    expect(host.textContent).not.toContain('Bad Gateway');
  });

  it('row 3: quote refused 400 validationFailed (and mapped refusal code), field amount in EN shows the EN worded text and keeps field "amount" under the amount input', async () => {
    setLang('en');
    const s = fakeServer();
    let refusalBody: { code: string; message: string; field: string } = {
      code: 'validationFailed',
      message: 'validationFailed',
      field: 'amount',
    };
    s.on('POST', '/api/quotes', () => ({
      status: 400,
      body: { error: refusalBody },
    }));
    const store = makeStore(s);
    await vi.waitFor(() => expect(store.config()).toBeDefined());
    const disposeView = render(() => rateStep.component({ store }), host);
    disposers.push(disposeView);

    store.setCurrency('USD');
    store.setAmount('abc');
    await store.requote();

    const expectedValidationText = wordedError(
      new ApiError({ status: 400, code: 'validationFailed', message: 'validationFailed', field: 'amount' }),
    );
    expect(store.quoteError()).toEqual({
      field: 'amount',
      message: expectedValidationText,
    });
    expect(store.quoteError()?.message).toBe(sharedEn['error.serverError']);
    expect(store.quoteError()?.message).not.toBe('validationFailed');

    const amountInput = host.querySelector<HTMLInputElement>('input[name="amount"]');
    expect(amountInput).not.toBeNull();
    const amountLabel = amountInput!.closest('label');
    expect(amountLabel?.querySelector('.desk-error')?.textContent).toBe(expectedValidationText);

    // Also verify a mapped 400 refusal code on field "amount" translates to EN and stays under the amount input
    refusalBody = {
      code: 'invalidRequest',
      message: 'คำขอไม่ถูกต้องจากเซิร์ฟเวอร์',
      field: 'amount',
    };
    await store.requote();
    expect(store.quoteError()).toEqual({
      field: 'amount',
      message: sharedEn['error.invalidRequest'],
    });
    expect(amountLabel?.querySelector('.desk-error')?.textContent).toBe(sharedEn['error.invalidRequest']);
  });

  it('row 5: a code with no text in the table falls back to the server message across save, quote and booking load in any language', async () => {
    const s = fakeServer();
    const fallbackMsg = 'ข้อความเฉพาะจากเซิร์ฟเวอร์ที่ไม่อยู่ในตาราง';
    s.on('POST', '/api/transactions', () => ({
      status: 400,
      body: { error: { code: 'unmappedDeskCode', message: fallbackMsg } },
    }));
    const store = await readyForSave(s);

    for (const l of ['th', 'en'] as const) {
      setLang(l);
      await expect(store.submit()).resolves.toBe('refused');
      expect(store.saveError()).toBe(fallbackMsg);
    }

    s.on('POST', '/api/quotes', () => ({
      status: 400,
      body: { error: { code: 'unmappedDeskCode', message: fallbackMsg, field: 'amount' } },
    }));
    for (const l of ['th', 'en'] as const) {
      setLang(l);
      await store.requote();
      expect(store.quoteError()).toEqual({ field: 'amount', message: fallbackMsg });
    }

    // And when the server echoes the unmapped code itself as message, wordedError's guard prevents leaking the raw code
    s.on('POST', '/api/quotes', () => ({
      status: 400,
      body: { error: { code: 'unmappedDeskCode', message: 'unmappedDeskCode', field: 'amount' } },
    }));
    setLang('th');
    await store.requote();
    expect(store.quoteError()).toEqual({ field: 'amount', message: sharedTh['error.serverError'] });
  });
});
