import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setAudience, setLang } from '../../../shared/i18n';
import { th } from './i18n';
import type { QueueRate, RequestFormValue } from './queue-api';
import { RequestForm, type RequestFormProps } from './RequestForm';

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let onSubmit: ReturnType<typeof vi.fn<(v: RequestFormValue) => void>>;

const DEFAULT_RATES: QueueRate[] = [
  { code: 'USD', name: 'ดอลลาร์', nameEn: 'US Dollar', buy: 32.1, sell: 32.6 },
];

function mount(props: Partial<RequestFormProps> = {}): void {
  dispose = render(
    () => (
      <RequestForm
        rates={props.rates ?? DEFAULT_RATES}
        busy={props.busy ?? false}
        onSubmit={props.onSubmit ?? onSubmit}
      />
    ),
    host,
  );
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
  host.remove();
});

describe('QW-2c: RequestForm', () => {
  it('row 1: rates = [USD, EUR] -> select[name="currency"] has one option per code (value = code, label = code and name)', () => {
    const rates: QueueRate[] = [
      { code: 'USD', name: 'ดอลลาร์', nameEn: 'US Dollar', buy: 32.1, sell: 32.6 },
      { code: 'EUR', name: 'ยูโร', nameEn: 'Euro', buy: 35.1, sell: 35.8 },
    ];
    mount({ rates });
    const select = host.querySelector<HTMLSelectElement>('select[name="currency"]');
    expect(select).not.toBeNull();
    const options = select!.querySelectorAll('option');
    expect(options).toHaveLength(2);
    expect(options[0].value).toBe('USD');
    expect(options[0].label).toContain('USD');
    expect(options[0].label).toContain('ดอลลาร์');
    expect(options[1].value).toBe('EUR');
    expect(options[1].label).toContain('EUR');
    expect(options[1].label).toContain('ยูโร');
  });

  it('row 2: render -> select[name="type"] has BUY and SELL (labels buy / sell); input[name="amount"] takes a decimal (inputmode="decimal"); button[data-action="submit-request"] labelled submit', () => {
    mount();
    const typeSelect = host.querySelector<HTMLSelectElement>('select[name="type"]');
    expect(typeSelect).not.toBeNull();
    const typeOptions = typeSelect!.querySelectorAll('option');
    expect(typeOptions).toHaveLength(2);
    expect(typeOptions[0].value).toBe('BUY');
    expect(typeOptions[0].textContent?.trim()).toBe(th.buy);
    expect(typeOptions[1].value).toBe('SELL');
    expect(typeOptions[1].textContent?.trim()).toBe(th.sell);

    const amountInput = host.querySelector<HTMLInputElement>('input[name="amount"]');
    expect(amountInput).not.toBeNull();
    expect(amountInput!.getAttribute('inputmode')).toBe('decimal');

    const submitButton = host.querySelector<HTMLButtonElement>('button[data-action="submit-request"]');
    expect(submitButton).not.toBeNull();
    expect(submitButton!.textContent?.trim()).toBe(th.submit);
  });

  it('row 3: pick SELL / EUR, type 250, press submit -> onSubmit({type:"SELL", currency:"EUR", amount:"250"}) once', () => {
    const rates: QueueRate[] = [
      { code: 'USD', name: 'ดอลลาร์', nameEn: 'US Dollar', buy: 32.1, sell: 32.6 },
      { code: 'EUR', name: 'ยูโร', nameEn: 'Euro', buy: 35.1, sell: 35.8 },
    ];
    mount({ rates });

    const type = host.querySelector<HTMLSelectElement>('select[name="type"]')!;
    type.value = 'SELL';
    type.dispatchEvent(new Event('change', { bubbles: true }));

    const cur = host.querySelector<HTMLSelectElement>('select[name="currency"]')!;
    cur.value = 'EUR';
    cur.dispatchEvent(new Event('change', { bubbles: true }));

    const amt = host.querySelector<HTMLInputElement>('input[name="amount"]')!;
    amt.value = '250';
    amt.dispatchEvent(new Event('input', { bubbles: true }));

    const button = host.querySelector<HTMLButtonElement>('button[data-action="submit-request"]')!;
    button.click();

    expect(onSubmit).toHaveBeenCalledTimes(1);
    expect(onSubmit).toHaveBeenCalledWith({
      type: 'SELL',
      currency: 'EUR',
      amount: '250',
    });
  });

  it('row 4: busy true -> the submit button is disabled and labelled submitting', () => {
    mount({ busy: true });
    const button = host.querySelector<HTMLButtonElement>('button[data-action="submit-request"]');
    expect(button).not.toBeNull();
    expect(button!.disabled).toBe(true);
    expect(button!.textContent?.trim()).toBe(th.submitting);
  });

  it('row 5: values set through change / input events -> onSubmit receives the picked values', () => {
    const rates: QueueRate[] = [
      { code: 'USD', name: 'ดอลลาร์', nameEn: 'US Dollar', buy: 32.1, sell: 32.6 },
      { code: 'JPY', name: 'เยน', nameEn: 'Yen', buy: 0.22, sell: 0.24 },
    ];
    mount({ rates });

    const type = host.querySelector<HTMLSelectElement>('select[name="type"]')!;
    type.value = 'BUY';
    type.dispatchEvent(new Event('change', { bubbles: true }));

    const cur = host.querySelector<HTMLSelectElement>('select[name="currency"]')!;
    cur.value = 'JPY';
    cur.dispatchEvent(new Event('change', { bubbles: true }));

    const amt = host.querySelector<HTMLInputElement>('input[name="amount"]')!;
    amt.value = '10000.50';
    amt.dispatchEvent(new Event('input', { bubbles: true }));

    const button = host.querySelector<HTMLButtonElement>('button[data-action="submit-request"]')!;
    button.click();

    expect(onSubmit).toHaveBeenCalledTimes(1);
    expect(onSubmit).toHaveBeenCalledWith({
      type: 'BUY',
      currency: 'JPY',
      amount: '10000.50',
    });
  });

  it('row 6: classes xc-queue-form, xc-queue-field, xc-queue-label, xc-queue-select, xc-queue-input, xc-queue-submit', () => {
    mount();
    expect(host.querySelector('.xc-queue-form')).not.toBeNull();
    expect(host.querySelectorAll('.xc-queue-field').length).toBeGreaterThanOrEqual(3);
    expect(host.querySelectorAll('.xc-queue-label').length).toBeGreaterThanOrEqual(3);
    expect(host.querySelectorAll('.xc-queue-select').length).toBe(2);
    expect(host.querySelector('.xc-queue-input')).not.toBeNull();
    expect(host.querySelector('.xc-queue-submit')).not.toBeNull();
  });
});
