// #4923 (T23) up-front tests: the queue request amount is checked as Money (at most 2 decimals) and at most
// 10,000,000 before a request is sent (design web-input-validation.md §3; kiosk_machine.rs:44-47).
// A refused amount shows a worded line at the field, `[data-field-error="amount"]`, in the person's language,
// and `onSubmit` is not called. A good amount is handed over unchanged, as today.
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setAudience, setLang } from '../../../shared/i18n';
import type { QueueRate, RequestFormValue } from './queue-api';
import { RequestForm } from './RequestForm';

const THAI_SCRIPT = /[฀-๿]/;

const RATES: QueueRate[] = [
  { code: 'USD', name: 'ดอลลาร์', nameEn: 'US Dollar', buy: 32.1, sell: 32.6 },
];

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let onSubmit: ReturnType<typeof vi.fn<(v: RequestFormValue) => void>>;

function mount(): void {
  dispose = render(() => <RequestForm rates={RATES} busy={false} onSubmit={onSubmit} />, host);
}

function typeAmount(value: string): void {
  const amt = host.querySelector<HTMLInputElement>('input[name="amount"]')!;
  amt.value = value;
  amt.dispatchEvent(new Event('input', { bubbles: true }));
}

function submit(): void {
  host.querySelector<HTMLButtonElement>('button[data-action="submit-request"]')!.click();
}

function amountLine(): string {
  return host.querySelector('[data-field-error="amount"]')?.textContent?.trim() ?? '';
}

beforeEach(() => {
  setAudience('customer');
  setLang('th');
  onSubmit = vi.fn();
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  setLang('th');
});

describe('#4923 T23: RequestForm amount is checked before a request is sent', () => {
  it('QR1: 10000000.01 (above the kiosk cap of 10,000,000) → a worded line at the amount, in th and en; no request', () => {
    mount();
    typeAmount('10000000.01');
    submit();

    expect(onSubmit).not.toHaveBeenCalled();
    expect(amountLine()).not.toBe('');
    expect(amountLine()).toMatch(THAI_SCRIPT);

    dispose?.();
    host.textContent = '';
    setLang('en');
    mount();
    typeAmount('10000000.01');
    submit();

    expect(onSubmit).not.toHaveBeenCalled();
    expect(amountLine()).not.toBe('');
    expect(amountLine()).not.toMatch(THAI_SCRIPT);
  });

  it('QR2: 1,000 (a comma is not a plain decimal) → a worded line at the amount; no request', () => {
    mount();
    typeAmount('1,000');
    submit();

    expect(onSubmit).not.toHaveBeenCalled();
    expect(amountLine()).not.toBe('');
    expect(amountLine()).toMatch(THAI_SCRIPT);
  });

  it('QR3: 500 → sent as today, with no line at the amount', () => {
    mount();
    typeAmount('500');
    submit();

    expect(onSubmit).toHaveBeenCalledTimes(1);
    expect(onSubmit).toHaveBeenCalledWith({ type: 'BUY', currency: 'USD', amount: '500' });
    expect(amountLine()).toBe('');
  });
});
