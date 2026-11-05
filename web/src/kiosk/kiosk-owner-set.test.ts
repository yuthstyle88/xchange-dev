// The kiosk's business values are owner-set data served by the server, never constants in kiosk.js
// (owner 2026-11-05: "ว่า อันไหน คอนฟิก ได้ อย่า ฝังใหน code"; api/src/application/kiosk_screen.rs).
import { beforeAll, describe, expect, it } from 'vitest';
import { el, fakeApi, kioskJs, openKiosk, ratesDrawn, settle, type KioskFetch } from './load-kiosk';

const chips = () => [...document.querySelectorAll('#quick-amounts button')].map((b) => b.textContent);

describe('kiosk.js', () => {
  it('takes its values from the server, keeping a fallback only for a server that sends none', () => {
    expect(kioskJs).toContain('let kioskValues={...NOT_SET_FALLBACK,...window.XC_KIOSK_SCREEN};');
    // the old constants are gone: the idle reset, the review hold and the refresh period read kioskValues
    expect(kioskJs).not.toMatch(/\b120000\b/);
    expect(kioskJs).not.toMatch(/seconds=3\b/);
    expect(kioskJs).not.toMatch(/},10000\)/);
  });
});

describe('the chips', () => {
  let answerScreen: object | undefined;
  const api: KioskFetch = async (url, init) => {
    const answer = await fakeApi()(url, init);
    if (url !== '/api/kiosk') return answer;
    const body = await answer.json();
    return new Response(JSON.stringify({ ...body, screen: answerScreen }), { status: 200 });
  };

  beforeAll(async () => {
    answerScreen = undefined;
    openKiosk('?branch=MAIN', api, { quickAmounts: [20, 50], tiles: 6, idleSeconds: 120, reviewSeconds: 3, refreshSeconds: 10 });
    await ratesDrawn();
  });

  it('are the page’s values before the branch answers', () => {
    expect(chips()).toEqual(['+20', '+50']);
  });

  it('become the branch’s own values from its rates answer', async () => {
    answerScreen = { quickAmounts: [200, 700], tiles: 6, idleSeconds: 120, reviewSeconds: 3, refreshSeconds: 10 };
    el<HTMLSelectElement>('branch-select'); // the page is up
    // the next load (a reset does one) reads the answer's values
    window.dispatchEvent(new Event('kiosk-attract-dismiss'));
    await settle();
    expect(chips()).toEqual(['+200', '+700']);
  });
});
