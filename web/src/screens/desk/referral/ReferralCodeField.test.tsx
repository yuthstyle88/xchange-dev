import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { ApiError, type ApiClient } from '../../../shared/api/client';
import { ReferralCodeField } from './ReferralCodeField';
import * as referralI18n from './i18n';
import { step as receiptStep } from '../steps/receipt/step';

function stubStore(outcome: 'idle' | 'saving' | 'saved' | 'refused' | 'unknown', txId = 'tx-saved-999') {
  const [outcomeSignal, setOutcome] = createSignal(outcome);
  return {
    path: { kind: 'staff', devicesLink: false },
    step: () => 4 as const,
    branch: () => 'MAIN',
    config: () => ({ shiftOpen: true, adminUrl: '/?branch=MAIN', paymentMethods: [] }),
    outcome: outcomeSignal,
    transactionState: () => (outcomeSignal() === 'saved' ? 'COMPLETED' : undefined),
    saveError: () => undefined,
    saved: () => (outcomeSignal() === 'saved' ? { id: txId, status: 'COMPLETED' as const, documentRef: 'DOC-123' } : undefined),
    submit: vi.fn(async () => 'saved'),
    backToEdit: vi.fn(),
    startNew: vi.fn(() => true),
    go: vi.fn(),
    api: undefined as unknown as ApiClient,
    _setOutcome: setOutcome,
  };
}

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
  setLang('th');
});

describe('ReferralCodeField (#983)', () => {
  it('shows only when outcome is saved (hidden before save)', () => {
    for (const outcome of ['idle', 'saving', 'refused', 'unknown'] as const) {
      const store = stubStore(outcome);
      dispose = render(() => receiptStep.component({ store: store as any }), host);
      expect(host.querySelector('.desk-referral-field'), `hidden on outcome=${outcome}`).toBeNull();
      dispose();
      dispose = undefined;
      host.innerHTML = '';
    }

    const store = stubStore('saved');
    dispose = render(() => receiptStep.component({ store: store as any }), host);
    expect(host.querySelector('.desk-referral-field'), 'visible on outcome=saved').not.toBeNull();
  });

  it('empty sends nothing', async () => {
    const mockApi = {
      command: vi.fn(),
    } as unknown as ApiClient;

    dispose = render(() => <ReferralCodeField transactionId="tx-1" api={mockApi} />, host);

    const button = host.querySelector<HTMLButtonElement>('.desk-referral-btn')!;
    button.click();

    expect(mockApi.command).not.toHaveBeenCalled();
  });

  it('ab12cd is sent as typed (the server normalises it)', async () => {
    const mockApi = {
      command: vi.fn().mockResolvedValue({ captureId: 'cap-1', status: 'ACTIVE' }),
    } as unknown as ApiClient;

    dispose = render(() => <ReferralCodeField transactionId="tx-1" api={mockApi} />, host);

    const input = host.querySelector<HTMLInputElement>('.desk-referral-input')!;
    input.value = 'ab12cd';
    input.dispatchEvent(new Event('input', { bubbles: true }));

    const button = host.querySelector<HTMLButtonElement>('.desk-referral-btn')!;
    button.click();

    expect(mockApi.command).toHaveBeenCalledTimes(1);
    expect(mockApi.command).toHaveBeenCalledWith(
      'POST',
      '/api/transactions/tx-1/referral',
      { id: 'tx-1', referralCode: 'ab12cd' },
      expect.any(String),
    );
  });

  it('calls attachReferral with the saved transaction id', async () => {
    const mockApi = {
      command: vi.fn().mockResolvedValue({ captureId: 'cap-1', status: 'ACTIVE' }),
    } as unknown as ApiClient;

    dispose = render(() => <ReferralCodeField transactionId="tx-saved-abc-123" api={mockApi} />, host);

    const input = host.querySelector<HTMLInputElement>('.desk-referral-input')!;
    input.value = 'BROKER99';
    input.dispatchEvent(new Event('input', { bubbles: true }));

    const button = host.querySelector<HTMLButtonElement>('.desk-referral-btn')!;
    button.click();

    expect(mockApi.command).toHaveBeenCalledWith(
      'POST',
      '/api/transactions/tx-saved-abc-123/referral',
      { id: 'tx-saved-abc-123', referralCode: 'BROKER99' },
      expect.any(String),
    );
  });

  it('one request per click, with an Idempotency-Key', async () => {
    let resolveCommand: (v: any) => void;
    const commandPromise = new Promise((r) => {
      resolveCommand = r;
    });
    const mockApi = {
      command: vi.fn().mockReturnValue(commandPromise),
    } as unknown as ApiClient;

    dispose = render(() => <ReferralCodeField transactionId="tx-1" api={mockApi} />, host);

    const input = host.querySelector<HTMLInputElement>('.desk-referral-input')!;
    input.value = 'BROKER123';
    input.dispatchEvent(new Event('input', { bubbles: true }));

    const button = host.querySelector<HTMLButtonElement>('.desk-referral-btn')!;
    // Rapid double click
    button.click();
    button.click();

    expect(mockApi.command).toHaveBeenCalledTimes(1);
    const key = (mockApi.command as any).mock.calls[0][3];
    expect(typeof key).toBe('string');
    expect(key.length).toBeGreaterThan(0);
    expect(key).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i);

    resolveCommand!({ captureId: 'cap-1', status: 'ACTIVE' });
  });

  it('shows 200 as "บันทึกแล้ว" with the code and locks the field', async () => {
    const mockApi = {
      command: vi.fn().mockResolvedValue({ captureId: 'cap-1', status: 'ACTIVE' }),
    } as unknown as ApiClient;

    dispose = render(() => <ReferralCodeField transactionId="tx-1" api={mockApi} />, host);

    const input = host.querySelector<HTMLInputElement>('.desk-referral-input')!;
    input.value = 'BROKER77';
    input.dispatchEvent(new Event('input', { bubbles: true }));

    const button = host.querySelector<HTMLButtonElement>('.desk-referral-btn')!;
    button.click();

    await new Promise((r) => setTimeout(r, 0));

    expect(host.textContent).toContain('บันทึกแล้ว');
    expect(host.textContent).toContain('BROKER77');
    expect(input.disabled).toBe(true);
    expect(button.disabled).toBe(true);
  });

  it('shows 409 as "รายการนี้มีโค้ดแนะนำแล้ว" and locks the field', async () => {
    const mockApi = {
      command: vi.fn().mockRejectedValue(
        new ApiError({
          status: 409,
          code: 'referralAlreadyCaptured',
          message: 'รายการนี้มีรหัสแนะนำแล้ว ให้ใช้การเปลี่ยนรหัสแทน',
        }),
      ),
    } as unknown as ApiClient;

    dispose = render(() => <ReferralCodeField transactionId="tx-1" api={mockApi} />, host);

    const input = host.querySelector<HTMLInputElement>('.desk-referral-input')!;
    input.value = 'BROKER88';
    input.dispatchEvent(new Event('input', { bubbles: true }));

    const button = host.querySelector<HTMLButtonElement>('.desk-referral-btn')!;
    button.click();

    await new Promise((r) => setTimeout(r, 0));

    expect(host.textContent).toContain('รายการนี้มีโค้ดแนะนำแล้ว');
    expect(input.disabled).toBe(true);
    expect(button.disabled).toBe(true);
  });

  it('shows 400 beside the field and does not lock the field', async () => {
    const mockApi = {
      command: vi.fn().mockRejectedValue(
        new ApiError({
          status: 400,
          code: 'validationFailed',
          message: 'รหัสแนะนำไม่ถูกต้อง',
        }),
      ),
    } as unknown as ApiClient;

    dispose = render(() => <ReferralCodeField transactionId="tx-1" api={mockApi} />, host);

    const input = host.querySelector<HTMLInputElement>('.desk-referral-input')!;
    input.value = 'BAD';
    input.dispatchEvent(new Event('input', { bubbles: true }));

    const button = host.querySelector<HTMLButtonElement>('.desk-referral-btn')!;
    button.click();

    await new Promise((r) => setTimeout(r, 0));

    expect(host.textContent).toContain('รหัสแนะนำไม่ถูกต้อง');
    expect(input.disabled).toBe(false);
    expect(button.disabled).toBe(false);
  });

  it('New and Print stay enabled whatever the referral field does', async () => {
    const mockApi = {
      command: vi.fn().mockRejectedValue(
        new ApiError({
          status: 400,
          code: 'validationFailed',
          message: 'รหัสแนะนำไม่ถูกต้อง',
        }),
      ),
    } as unknown as ApiClient;

    const store = stubStore('saved');
    store.api = mockApi;

    dispose = render(() => receiptStep.component({ store: store as any }), host);

    const buttons = () => [...host.querySelectorAll<HTMLButtonElement>('button')];
    const newBtn = () => buttons().find((b) => (b.textContent ?? '').includes('ทำรายการใหม่'));
    const printBtn = () => buttons().find((b) => (b.textContent ?? '').includes('พิมพ์ใบเสร็จ'));

    expect(newBtn()?.disabled).toBe(false);
    expect(printBtn()?.disabled).toBe(false);

    // Enter bad code and click save
    const input = host.querySelector<HTMLInputElement>('.desk-referral-input')!;
    input.value = 'INVALID';
    input.dispatchEvent(new Event('input', { bubbles: true }));

    const saveRefBtn = buttons().find((b) => (b.textContent ?? '').includes('บันทึกโค้ดแนะนำ'))!;
    saveRefBtn.click();

    await new Promise((r) => setTimeout(r, 0));

    expect(newBtn()?.disabled).toBe(false);
    expect(printBtn()?.disabled).toBe(false);
  });

  it('no text says "real time" or "live"', () => {
    const allTh = JSON.stringify(referralI18n.th);
    const allEn = JSON.stringify(referralI18n.en);
    expect(allTh.toLowerCase()).not.toMatch(/real.?time|\blive\b|เรียลไทม์/);
    expect(allEn.toLowerCase()).not.toMatch(/real.?time|\blive\b|เรียลไทม์/);
  });

  it('supports English', async () => {
    setLang('en');
    const mockApi = {
      command: vi.fn().mockResolvedValue({ captureId: 'cap-1', status: 'ACTIVE' }),
    } as unknown as ApiClient;

    dispose = render(() => <ReferralCodeField transactionId="tx-1" api={mockApi} />, host);

    expect(host.textContent).toContain('Referral Code (Optional)');
    expect(host.textContent).toContain('Save referral code');

    const input = host.querySelector<HTMLInputElement>('.desk-referral-input')!;
    input.value = 'BROKER1';
    input.dispatchEvent(new Event('input', { bubbles: true }));

    const button = host.querySelector<HTMLButtonElement>('.desk-referral-btn')!;
    button.click();

    await new Promise((r) => setTimeout(r, 0));

    expect(host.textContent).toContain('Saved');
    expect(host.textContent).toContain('BROKER1');
  });
});
