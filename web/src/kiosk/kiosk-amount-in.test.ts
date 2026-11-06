// R6 / F27 (#564): on SELL (the customer gives baht, gets foreign currency) the kiosk lets the customer type the foreign
// amount they want. Owner ruling R6 on #564, issuecomment-6673699296 (2026-11-06 07:56 ICT): "Add a foreign-amount entry
// on the kiosk." Symptom (issuecomment-6657891388, F27): a customer who wants USD 300 had to type THB 10,209.
//
// Written before the implementation by the ticket author (claude/33691c06). Do not edit, delete or weaken; add tests in
// a file of your own.
//
// Pinned here:
// - on SELL, two buttons beside the amount, `[data-amount-in="THB"]` and `[data-amount-in="FOREIGN"]`, with
//   `aria-pressed` saying which one is chosen; THB is chosen whenever SELL is chosen. On BUY neither shows;
// - FOREIGN: the label and the field's code name the foreign currency, the estimate is the baht to pay
//   (amount × sell rate, to the satang), the quick amounts (baht) are hidden, a typed amount too small to buy one cent
//   in baht is still a valid amount, and the quote sends `amountIn: "FOREIGN"` with the typed amount;
// - THB (and BUY): the quote payload is today's, byte for byte, with no `amountIn`;
// - changing between THB and FOREIGN clears the amount, so 300 baht never silently becomes 300 dollars.
// The fake API (load-kiosk.ts) sells the dollar at 34.1.
import { beforeAll, describe, expect, it } from 'vitest';
import { el, fakeApi, openKiosk, ratesDrawn, settle, type KioskFetch } from './load-kiosk';

const sent: Array<Record<string, unknown>> = [];
const json = (body: unknown) =>
  new Response(JSON.stringify(body), { status: 200, headers: { 'content-type': 'application/json' } });

function withQuotes(base: KioskFetch): KioskFetch {
  return async (url, init) => {
    const path = new URL(url, 'http://kiosk.test').pathname;
    if (path === '/api/kiosk') {
      const body = JSON.parse(String(init?.body ?? '{}')) as { action: string; payload: Record<string, unknown> };
      if (body.action === 'quote') {
        sent.push(body.payload);
        const p = body.payload;
        return json({
          id: `00000000-0000-4000-8000-0000000f27${String(sent.length).padStart(2, '0')}`,
          queue: 'K-00000000',
          orderNo: 'ORD-00000000',
          transactionNo: null,
          status: 'QUOTED',
          paymentMethod: 'Cash',
          type: p.type,
          currency: 'USD',
          rate: p.type === 'BUY' ? 33.53 : 34.1,
          foreign: 300,
          thb: 10230,
          inputAmount: Number(p.amount),
          branchName: 'Main branch',
        });
      }
    }
    return base(url, init);
  };
}

const toggle = (value: 'THB' | 'FOREIGN') => document.querySelector<HTMLButtonElement>(`[data-amount-in="${value}"]`);
const shown = (node: Element | null) => !!node && !node.closest('[hidden]');
/** The THB or FOREIGN button, which must exist. */
const choice = (value: 'THB' | 'FOREIGN') => {
  const found = toggle(value);
  expect(found, `a [data-amount-in="${value}"] button`).not.toBeNull();
  return found!;
};
const key = (label: string) =>
  [...document.querySelectorAll<HTMLButtonElement>('#amount-keypad button')].find((b) => b.textContent === label)!;
const amount = () => el<HTMLInputElement>('amount').value;
const clear = () => {
  while (amount() !== '') key('⌫').click();
};
const type = (digits: string) => {
  clear();
  for (const d of digits) key(d).click();
  expect(amount()).toBe(digits);
};
/** Presses Review and returns the payload of the quote it sent (or undefined when none was sent). */
async function review(): Promise<Record<string, unknown> | undefined> {
  const before = sent.length;
  el<HTMLButtonElement>('review').click();
  await settle();
  const payload = sent.length > before ? sent[sent.length - 1] : undefined;
  if (payload) {
    expect(el('confirm').hidden).toBe(false);
    el<HTMLButtonElement>('back').click();
  }
  expect(el('select').hidden).toBe(false);
  return payload;
}
const sell = () => el<HTMLButtonElement>('sell').click();
const buy = () => el<HTMLButtonElement>('buy').click();

describe('foreign-amount entry on SELL', () => {
  beforeAll(async () => {
    openKiosk('?branch=MAIN', withQuotes(fakeApi()));
    await ratesDrawn();
  });

  it('BUY shows no THB / FOREIGN choice', () => {
    expect(el('buy').getAttribute('aria-pressed')).toBe('true');
    expect(shown(toggle('THB'))).toBe(false);
    expect(shown(toggle('FOREIGN'))).toBe(false);
  });

  it('SELL shows the two buttons, THB chosen, and the quick amounts', () => {
    sell();
    expect(shown(toggle('THB'))).toBe(true);
    expect(shown(toggle('FOREIGN'))).toBe(true);
    expect(choice('THB').getAttribute('aria-pressed')).toBe('true');
    expect(choice('FOREIGN').getAttribute('aria-pressed')).toBe('false');
    expect(el('amount-code').textContent).toBe('THB');
    expect(el('amount-label').textContent).toContain('THB');
    expect(shown(el('quick-amounts'))).toBe(true);
  });

  it('THB sends today’s payload, byte for byte, with no amountIn', async () => {
    sell();
    type('1000');
    const payload = await review();
    expect(JSON.stringify(payload)).toBe(
      '{"type":"SELL","currency":"USD","amount":"1000","paymentMethod":"Cash","branchId":"MAIN"}',
    );
  });

  it('choosing FOREIGN clears the amount, names the dollar, and hides the baht quick amounts', () => {
    sell();
    type('300');
    choice('FOREIGN').click();
    expect(choice('FOREIGN').getAttribute('aria-pressed')).toBe('true');
    expect(choice('THB').getAttribute('aria-pressed')).toBe('false');
    expect(amount()).toBe('');
    expect(el('amount-code').textContent).toBe('USD');
    expect(el('amount-label').textContent).toContain('USD');
    expect(el('amount-label').textContent).not.toContain('THB');
    expect(shown(el('quick-amounts'))).toBe(false);
  });

  it('FOREIGN estimates the baht to pay: 300 × 34.1 = 10,230.00 THB', () => {
    type('300');
    expect(el('estimate').textContent).toContain('10,230.00 THB');
  });

  it('FOREIGN sends amountIn "FOREIGN" with the typed amount', async () => {
    type('300');
    const payload = await review();
    expect(payload).toEqual({
      type: 'SELL',
      currency: 'USD',
      amount: '300',
      paymentMethod: 'Cash',
      amountIn: 'FOREIGN',
      branchId: 'MAIN',
    });
    expect(choice('FOREIGN').getAttribute('aria-pressed')).toBe('true');
  });

  it('FOREIGN takes one cent of a dollar (it would buy no cent as baht)', async () => {
    choice('FOREIGN').click();
    clear();
    key('0').click();
    key('.').click();
    key('0').click();
    key('1').click();
    expect(amount()).toBe('0.01');
    const payload = await review();
    expect(payload).toMatchObject({ type: 'SELL', amount: '0.01', amountIn: 'FOREIGN' });
  });

  it('back to THB clears the amount and shows the quick amounts again', () => {
    choice('FOREIGN').click();
    type('300');
    choice('THB').click();
    expect(choice('THB').getAttribute('aria-pressed')).toBe('true');
    expect(amount()).toBe('');
    expect(el('amount-code').textContent).toBe('THB');
    expect(shown(el('quick-amounts'))).toBe(true);
  });

  it('BUY hides the choice and sends today’s payload; SELL again starts on THB', async () => {
    choice('FOREIGN').click();
    buy();
    expect(shown(toggle('THB'))).toBe(false);
    expect(shown(toggle('FOREIGN'))).toBe(false);
    expect(el('amount-code').textContent).toBe('USD');
    type('100');
    const payload = await review();
    expect(JSON.stringify(payload)).toBe(
      '{"type":"BUY","currency":"USD","amount":"100","paymentMethod":"Cash","branchId":"MAIN"}',
    );
    sell();
    expect(choice('THB').getAttribute('aria-pressed')).toBe('true');
    expect(choice('FOREIGN').getAttribute('aria-pressed')).toBe('false');
    expect(el('amount-code').textContent).toBe('THB');
  });
});
