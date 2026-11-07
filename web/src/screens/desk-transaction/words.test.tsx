// F53 (#6715): worded status, side, method, rate and quantity in desk-transaction detail.
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../shared/i18n';
import {
  makeTransactionDetailDto,
  makeTxLineDto,
  makeTxPaymentDto,
  type TransactionDetailDto,
  type TxPaymentDto,
} from '../../shared/testing/fixtures';
import { HeaderSection, statusName, statusText } from './HeaderSection';
import { formatQuantity, LinesSection, sideName } from './LinesSection';
import { methodName, PaymentSection } from './PaymentSection';
import { formatRate } from '../../shared/ui/rate';
import { formatMoney } from '../../shared/ui/MoneyText';

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  setLang('th');
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  setLang('th');
});

function text(el: Element): string {
  return (el.textContent ?? '').replace(/\s+/g, ' ').trim();
}

describe('F53 status wording (th and en)', () => {
  it('words COMPLETED, CANCELLED, and pending in Thai', () => {
    setLang('th');
    expect(statusName('COMPLETED')).toBe('สำเร็จ');
    expect(statusName('CANCELLED')).toBe('ยกเลิก');
    expect(statusName('PENDING')).toBe('รอบันทึกเอกสาร');
    expect(statusName('PENDING_APPROVAL')).toBe('รอบันทึกเอกสาร');
    expect(statusName('OTHER')).toBe('รอบันทึกเอกสาร');
  });

  it('words COMPLETED, CANCELLED, and pending in English', () => {
    setLang('en');
    expect(statusName('COMPLETED')).toBe('Completed');
    expect(statusName('CANCELLED')).toBe('Cancelled');
    expect(statusName('PENDING')).toBe('Pending');
    expect(statusName('PENDING_APPROVAL')).toBe('Pending');
    expect(statusName('OTHER')).toBe('Pending');
  });

  it('statusText preserves reversal suffixes with worded status', () => {
    const tx = (over: Partial<TransactionDetailDto> = {}) =>
      makeTransactionDetailDto({
        status: 'COMPLETED',
        reversal: null,
        ...over,
      });

    setLang('th');
    expect(statusText(tx())).toBe('สำเร็จ');
    expect(statusText(tx({ reversal: { status: 'POSTED', postedAt: '2026-11-01T05:44:00Z' } as any }))).toBe(
      'สำเร็จ · กลับรายการแล้ว 01/11/2026 12:44',
    );
    expect(statusText(tx({ reversal: { status: 'POSTED', postedAt: null } as any }))).toBe('สำเร็จ · กลับรายการแล้ว');
    expect(statusText(tx({ reversal: { status: 'REQUESTED' } as any }))).toBe('สำเร็จ · รอกลับรายการ');
    expect(statusText(tx({ reversal: { status: 'APPROVED' } as any }))).toBe('สำเร็จ · รอกลับรายการ');

    setLang('en');
    expect(statusText(tx())).toBe('Completed');
    expect(statusText(tx({ reversal: { status: 'POSTED', postedAt: '2026-11-01T05:44:00Z' } as any }))).toBe(
      'Completed · Reversed 01/11/2026 12:44',
    );
    expect(statusText(tx({ reversal: { status: 'POSTED', postedAt: null } as any }))).toBe('Completed · Reversed');
    expect(statusText(tx({ reversal: { status: 'REQUESTED' } as any }))).toBe('Completed · Reversal pending');
  });

  it('HeaderSection renders the worded status in dl', () => {
    const tx = makeTransactionDetailDto({ status: 'COMPLETED' });
    setLang('th');
    dispose = render(() => <HeaderSection tx={tx} />, host);
    expect(text(host)).toContain('สำเร็จ');

    dispose();
    setLang('en');
    dispose = render(() => <HeaderSection tx={tx} />, host);
    expect(text(host)).toContain('Completed');
  });
});

describe('F53 side wording (th and en)', () => {
  it('words BUY and SELL in Thai and English', () => {
    setLang('th');
    expect(sideName('BUY')).toBe('ร้านรับซื้อ');
    expect(sideName('SELL')).toBe('ร้านขาย');

    setLang('en');
    expect(sideName('BUY')).toBe('Buy');
    expect(sideName('SELL')).toBe('Sell');
  });

  it('LinesSection renders worded side in table', () => {
    const lines = [
      makeTxLineDto({ lineNo: 1, side: 'BUY' }),
      makeTxLineDto({ lineNo: 2, side: 'SELL' }),
    ];

    setLang('th');
    dispose = render(() => <LinesSection lines={lines} />, host);
    const rowsTh = host.querySelectorAll('tbody tr');
    expect(text(rowsTh[0]!.querySelectorAll('td')[1]!)).toBe('ร้านรับซื้อ');
    expect(text(rowsTh[1]!.querySelectorAll('td')[1]!)).toBe('ร้านขาย');

    dispose();
    setLang('en');
    dispose = render(() => <LinesSection lines={lines} />, host);
    const rowsEn = host.querySelectorAll('tbody tr');
    expect(text(rowsEn[0]!.querySelectorAll('td')[1]!)).toBe('Buy');
    expect(text(rowsEn[1]!.querySelectorAll('td')[1]!)).toBe('Sell');
  });
});

describe('F53 method wording (th and en)', () => {
  it('words known and unknown methods in Thai', () => {
    setLang('th');
    expect(methodName('Cash')).toBe('เงินสด');
    expect(methodName('Credit Card')).toBe('บัตรเครดิต');
    expect(methodName('Alipay')).toBe('Alipay');
    expect(methodName('PromptPay')).toBe('พร้อมเพย์');
    expect(methodName('Bank Transfer')).toBe('โอนธนาคาร');
    expect(methodName('SCB Transfer')).toBe('โอนธนาคาร');
    expect(methodName('Crypto')).toBe('อื่น ๆ');
    expect(methodName('Unknown')).toBe('อื่น ๆ');
  });

  it('words known and unknown methods in English', () => {
    setLang('en');
    expect(methodName('Cash')).toBe('Cash');
    expect(methodName('Credit Card')).toBe('Credit Card');
    expect(methodName('Alipay')).toBe('Alipay');
    expect(methodName('PromptPay')).toBe('PromptPay');
    expect(methodName('Bank Transfer')).toBe('Bank Transfer');
    expect(methodName('SCB Transfer')).toBe('Bank Transfer');
    expect(methodName('Crypto')).toBe('Other');
    expect(methodName('Unknown')).toBe('Other');
  });

  it('PaymentSection renders worded method in field', () => {
    const payment: TxPaymentDto = makeTxPaymentDto({ method: 'Cash' });
    setLang('th');
    dispose = render(() => <PaymentSection payment={payment} />, host);
    expect(text(host)).toContain('เงินสด');

    dispose();
    setLang('en');
    dispose = render(() => <PaymentSection payment={payment} />, host);
    expect(text(host)).toContain('Cash');
  });
});

describe('F53 rate formatting through formatRate', () => {
  it('formats rate with 2-8 decimal places and unit size', () => {
    expect(formatRate('33.57000000', '1.0000', 'USD')).toBe('33.57');
    expect(formatRate('35.12000000', 1, 'USD')).toBe('35.12');
    expect(formatRate('0.24500000', 100, 'JPY')).toBe('0.245 / 100 JPY');
    expect(formatRate('0.24500000', '100.0000', 'JPY')).toBe('0.245 / 100 JPY');
    expect(formatRate('1.23456780', 1, 'EUR')).toBe('1.2345678');
  });

  it('LinesSection renders formatted rate in table', () => {
    const lines = [
      makeTxLineDto({
        lineNo: 1,
        currency: 'USD',
        unitSize: '1.0000',
        rateSnapshot: '33.57000000',
      }),
      makeTxLineDto({
        lineNo: 2,
        currency: 'JPY',
        unitSize: '100.0000',
        rateSnapshot: '0.24500000',
      }),
    ];
    dispose = render(() => <LinesSection lines={lines} />, host);
    const rows = host.querySelectorAll('tbody tr');
    expect(text(rows[0]!.querySelectorAll('td')[4]!)).toBe('33.57');
    expect(text(rows[1]!.querySelectorAll('td')[4]!)).toBe('0.245 / 100 JPY');
  });
});

describe('F53 quantity formatting through formatMoney with trimmed trailing zeros', () => {
  it('trims trailing zeros beyond 2 decimals and formats with commas', () => {
    expect(formatQuantity('100.0000')).toBe('100.00');
    expect(formatQuantity('1000.0000')).toBe('1,000.00');
    expect(formatQuantity('1234567.8900')).toBe('1,234,567.89');
    expect(formatQuantity('100.12340')).toBe('100.1234');
    expect(formatQuantity('100.12000')).toBe('100.12');
    expect(formatQuantity('1000')).toBe(formatMoney('1000'));
    expect(formatQuantity('100.5')).toBe('100.5');
  });

  it('LinesSection renders formatted quantity in table', () => {
    const lines = [
      makeTxLineDto({ lineNo: 1, quantity: '1000.0000' }),
      makeTxLineDto({ lineNo: 2, quantity: '100.0000' }),
    ];
    dispose = render(() => <LinesSection lines={lines} />, host);
    const rows = host.querySelectorAll('tbody tr');
    expect(text(rows[0]!.querySelectorAll('td')[2]!)).toBe('1,000.00');
    expect(text(rows[1]!.querySelectorAll('td')[2]!)).toBe('100.00');
  });
});
