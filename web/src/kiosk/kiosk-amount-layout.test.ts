// The amount area as the owner approved it on 2026-11-05 ("ไม่เปลี่ยน", "ผมว่ามัน ok"), top to bottom: the amount
// field at full width (code on the left, the number on the right), one row of the quick amounts, the complete keypad,
// then the estimate. The order is locked here; the look (full width, code left, number right, 4 chips in a row) is
// locked by api/tests/pages_kiosk_test.rs, which reads kiosk-xchange.css.
import { beforeAll, describe, expect, it } from 'vitest';
import { el, fakeApi, openKiosk, ratesDrawn } from './load-kiosk';

const before = (a: Element, b: Element) => Boolean(a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING);

describe('the amount area', () => {
  beforeAll(async () => {
    openKiosk('?branch=MAIN', fakeApi());
    await ratesDrawn();
  });

  it('is label, field, quick amounts, keypad, estimate, in that order', () => {
    const order = [
      el('amount-label'),
      document.querySelector('.amount-field')!,
      el('quick-amounts'),
      el('amount-keypad'),
      el('estimate'),
    ];
    for (let i = 1; i < order.length; i++) expect(before(order[i - 1], order[i])).toBe(true);
    expect(el('amount').parentElement!.className).toBe('amount-field');
    expect(el('amount-code').parentElement!.className).toBe('amount-field');
    expect(el('amount-code').textContent).toBe('USD');
  });

  it('has the complete keypad', () => {
    expect([...el('amount-keypad').children].map((b) => b.textContent).join(' ')).toBe('1 2 3 4 5 6 7 8 9 . 0 ⌫');
  });
});
