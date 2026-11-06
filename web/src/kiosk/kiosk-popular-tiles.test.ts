// The tiles: the branch's popular currencies first (`popular`, worked out on the server from its trading history),
// then the rest in the answer's order, up to the owner-set number of tiles. A search shows every match; the dropdown
// keeps every currency.
import { beforeAll, describe, expect, it } from 'vitest';
import { el, openKiosk, PAGE_SCREEN, ratesDrawn, type KioskFetch } from './load-kiosk';

const CODES = ['USD', 'EUR', 'JPY', 'CNY', 'GBP', 'SGD', 'KRW', 'HKD'];
let popular: string[] = [];

const api: KioskFetch = async (url, init) => {
  if (url === '/api/kiosk' && JSON.parse(String(init?.body)).action === 'rates') {
    return new Response(
      JSON.stringify({
        branchId: 'MAIN',
        branchName: 'Main branch',
        open: true,
        rates: CODES.map((code) => ({ code, name: code, nameEn: code, countries: [], buy: 10, sell: 11 })),
        paymentMethods: [{ id: 'Cash', enabled: true, buy: true, sell: true, instruction: '' }],
        popular,
        screen: PAGE_SCREEN,
      }),
      { status: 200 },
    );
  }
  return new Response('{}', { status: 404 });
};

const tiles = () => [...document.querySelectorAll('#currency-shortcuts button bdi')].map((b) => b.textContent);
const reload = async () => {
  window.dispatchEvent(new Event('kiosk-attract-dismiss'));
  await new Promise((r) => setTimeout(r, 50));
};

describe('currency tiles by popularity', () => {
  beforeAll(async () => {
    popular = ['JPY', 'GBP', 'USD'];
    openKiosk('?branch=MAIN', api);
    await ratesDrawn();
  });

  it('show the popular ones first, then the answer’s order, six in all', () => {
    expect(tiles()).toEqual(['JPY', 'GBP', 'USD', 'EUR', 'CNY', 'SGD']);
  });

  it('keep every currency in the dropdown', () => {
    expect([...el<HTMLSelectElement>('currency').options].map((o) => o.value)).toEqual(CODES);
  });

  it('show every match while searching', () => {
    const search = el<HTMLInputElement>('currency-search');
    search.value = 'dollar';
    search.dispatchEvent(new Event('input'));
    expect(tiles()).toEqual(['USD', 'SGD', 'HKD']);
    search.value = '';
    search.dispatchEvent(new Event('input'));
  });

  it('with no history, keep the answer’s order', async () => {
    popular = [];
    await reload();
    expect(tiles()).toEqual(['USD', 'EUR', 'JPY', 'CNY', 'GBP', 'SGD']);
  });
});
