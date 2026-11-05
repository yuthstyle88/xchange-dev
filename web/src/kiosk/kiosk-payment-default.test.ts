// The payment method is preselected (owner 2026-11-05: "Payment method ให้ default"): the branch's owner-set default
// when the side allows it, else Cash, else the side's first enabled method in the answer's order. No empty choice.
import { describe, expect, it } from 'vitest';
import { el, openKiosk, PAGE_SCREEN, ratesDrawn, type KioskFetch } from './load-kiosk';

const METHODS = [
  { id: 'Cash', enabled: true, buy: true, sell: false, instruction: '' },
  { id: 'Credit Card', enabled: false, buy: true, sell: true, instruction: '' },
  { id: 'Alipay', enabled: true, buy: false, sell: true, instruction: '' },
  { id: 'PromptPay', enabled: true, buy: true, sell: true, instruction: '' },
  { id: 'Bank Transfer', enabled: true, buy: true, sell: true, instruction: '' },
];

const api =
  (screen: object, methods = METHODS): KioskFetch =>
  async (url, init) => {
    if (url === '/api/kiosk' && JSON.parse(String(init?.body)).action === 'rates') {
      return new Response(
        JSON.stringify({
          branchId: 'MAIN',
          branchName: 'Main branch',
          open: true,
          rates: [{ code: 'USD', name: 'USD', nameEn: 'USD', countries: [], buy: 33, sell: 34 }],
          paymentMethods: methods,
          screen,
        }),
        { status: 200 },
      );
    }
    return new Response('{}', { status: 404 });
  };

const method = () => el<HTMLSelectElement>('payment-method').value;
const side = (id: 'buy' | 'sell') => el<HTMLButtonElement>(id).click();

describe('the payment method', () => {
  it('has no empty "Choose a method" choice', () => {
    openKiosk('?branch=MAIN', api(PAGE_SCREEN));
    expect([...el<HTMLSelectElement>('payment-method').options].map((o) => o.value)).not.toContain('');
  });

  it('opens on Cash for BUY, and takes the side’s first enabled method where Cash is off', async () => {
    openKiosk('?branch=MAIN', api(PAGE_SCREEN));
    await ratesDrawn();
    expect(method()).toBe('Cash');
    side('sell'); // Cash does not sell: the first enabled SELL method, in the answer's order
    expect(method()).toBe('Alipay');
    side('buy');
    expect(method()).toBe('Cash');
  });
});
