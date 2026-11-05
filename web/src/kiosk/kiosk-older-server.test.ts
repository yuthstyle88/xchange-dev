// A server that sends no screen values at all (an older one): the kiosk still shows the default chips,
// +100 · +500 · +1,000 · +5,000 (owner 2026-11-05: "ให้ default ตามนี้ ก่อน").
import { beforeAll, describe, expect, it } from 'vitest';
import { fakeApi, openKiosk, ratesDrawn } from './load-kiosk';

describe('no screen values from the server', () => {
  beforeAll(async () => {
    openKiosk('?branch=MAIN', fakeApi(), {});
    await ratesDrawn();
  });

  it('shows the default chips', () => {
    expect([...document.querySelectorAll('#quick-amounts button')].map((b) => b.textContent)).toEqual([
      '+100',
      '+500',
      '+1,000',
      '+5,000',
    ]);
  });
});
