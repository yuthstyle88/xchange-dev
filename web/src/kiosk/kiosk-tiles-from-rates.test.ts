// The currency tiles are the kiosk `rates` answer of the branch (owner 2026-11-05: "ต้อง link จาก db นะ"):
// its currencies, in its order, never a list of the page's own. ASK (listed by the server apart, as `ask`, never
// quoted: quote.rs refuses ASK off the desk) is found by search and says "Please contact staff"; HIDE never shows.
import { beforeAll, describe, expect, it } from 'vitest';
import { el, openKiosk, ratesDrawn, type KioskFetch } from './load-kiosk';

const rate = (code: string, extra: Record<string, unknown> = {}) => ({
  code,
  name: code,
  nameEn: code,
  countries: [],
  buy: 10,
  sell: 11,
  ...extra,
});

// the server prints SHOW currencies as `rates` and ASK ones apart as `ask` (no prices); HIDE ones are in neither
let answer = [rate('USD'), rate('TWD')];
const ask = [{ code: 'MYR', name: 'MYR', nameEn: 'MYR', countries: [] }];

const api: KioskFetch = async (url, init) => {
  if (url === '/api/kiosk' && JSON.parse(String(init?.body)).action === 'rates') {
    return new Response(
      JSON.stringify({
        branchId: 'MAIN',
        branchName: 'Main branch',
        open: true,
        rates: answer,
        ask,
        paymentMethods: [{ id: 'Cash', enabled: true, buy: true, sell: true, instruction: '' }],
      }),
      { status: 200 },
    );
  }
  return new Response('{}', { status: 404 });
};

const tiles = () =>
  [...document.querySelectorAll<HTMLButtonElement>('#currency-shortcuts button')].map((b) => ({
    code: b.querySelector('bdi')?.textContent,
    contact: b.querySelector('small')?.textContent ?? null,
  }));

describe('currency tiles', () => {
  beforeAll(async () => {
    openKiosk('?branch=MAIN', api);
    await ratesDrawn();
  });

  it('are the answer’s quotable currencies: USD and TWD; GBP (HIDE) never shows', () => {
    expect(tiles()).toEqual([
      { code: 'USD', contact: null },
      { code: 'TWD', contact: null },
    ]);
    expect([...el<HTMLSelectElement>('currency').options].map((o) => o.value)).toEqual(['USD', 'TWD', 'MYR']);
  });

  it('an ASK currency is found by search, says "Please contact staff" and cannot be reviewed', () => {
    const search = el<HTMLInputElement>('currency-search');
    search.value = 'MYR';
    search.dispatchEvent(new Event('input'));
    expect(tiles()).toEqual([{ code: 'MYR', contact: 'Please contact staff' }]);
    document.querySelector<HTMLButtonElement>('#currency-shortcuts button')!.click();
    expect(el<HTMLSelectElement>('currency').value).toBe('MYR');
    expect(el('review').textContent).toBe('Please contact staff');
    expect(el<HTMLButtonElement>('review').disabled).toBe(true);
    search.value = '';
    search.dispatchEvent(new Event('input'));
    el<HTMLSelectElement>('currency').value = 'USD';
    el('currency').dispatchEvent(new Event('change'));
  });

  it('a currency the admin turns on appears on the next rates refresh', async () => {
    answer = [rate('USD'), rate('TWD'), rate('JPY')];
    await new Promise((r) => setTimeout(r, 10_500));
    expect(tiles().map((t) => t.code)).toEqual(['USD', 'TWD', 'JPY']);
  }, 15_000);
});
