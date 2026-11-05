// An owner-set default payment method wins over Cash; with no method for a side the kiosk words "contact staff".
import { describe, expect, it } from 'vitest';
import { el, openKiosk, PAGE_SCREEN, ratesDrawn, type KioskFetch } from './load-kiosk';

const methods = [
  { id: 'Cash', enabled: true, buy: true, sell: true, instruction: '' },
  { id: 'PromptPay', enabled: true, buy: true, sell: false, instruction: '' },
];

const api: KioskFetch = async (url, init) => {
  if (url === '/api/kiosk' && JSON.parse(String(init?.body)).action === 'rates') {
    return new Response(
      JSON.stringify({
        branchId: 'MAIN',
        branchName: 'Main branch',
        open: true,
        rates: [{ code: 'USD', name: 'USD', nameEn: 'USD', countries: [], buy: 33, sell: 34 }],
        paymentMethods: methods,
        screen: { ...PAGE_SCREEN, defaultPaymentMethod: 'PromptPay', set: true, source: 'owner' },
      }),
      { status: 200 },
    );
  }
  return new Response('{}', { status: 404 });
};

describe('an owner-set default payment method', () => {
  it('overrides Cash, and gives way to Cash on a side that does not allow it', async () => {
    openKiosk('?branch=MAIN', api);
    await ratesDrawn();
    expect(el<HTMLSelectElement>('payment-method').value).toBe('PromptPay');
    el<HTMLButtonElement>('sell').click();
    expect(el<HTMLSelectElement>('payment-method').value).toBe('Cash');
  });

  it('with no method for the side, shows "contact staff" instead of an empty select', async () => {
    methods[0].sell = false;
    window.dispatchEvent(new Event('kiosk-attract-dismiss')); // a reset loads the rates again
    await new Promise((r) => setTimeout(r, 50));
    el<HTMLButtonElement>('sell').click();
    expect(el<HTMLSelectElement>('payment-method').hidden).toBe(true);
    expect(el('payment-instruction').textContent).toBe('Please contact staff');
    expect(el<HTMLButtonElement>('review').disabled).toBe(true);
  });
});
