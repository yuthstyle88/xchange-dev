// Test harness for the vendored customer kiosk (`api/static/kiosk/`, served at `/kiosk` by `api/src/presentation/pages.rs`).
// It is plain browser script, not a module of this app: the harness puts kiosk.html's body into this test file's jsdom
// document and runs the scripts in their kiosk.html order, as global scripts, with `fetch` answered by the caller.
// kiosk-attract.js (the 36 s loop) is not run here; kiosk.js only calls it as `window.KioskAttract?.show()`.
// One kiosk per test file: vitest gives each file its own jsdom.
import kioskHtml from '../../../api/static/kiosk/kiosk.html?raw';
import kioskI18n from '../../../api/static/kiosk/kiosk-i18n.js?raw';
import kioskExtras from '../../../api/static/kiosk/kiosk-language-extras.js?raw';
import kioskJs from '../../../api/static/kiosk/kiosk.js?raw';

export { kioskJs };

export type KioskFetch = (url: string, init?: RequestInit) => Promise<Response>;

const run = (code: string) => (0, eval)(code);

/** The i18n tables alone, as the page has them after its two text scripts ran. */
export function loadTexts(): {
  langs: Array<{ code: string }>;
  text: Record<string, Record<string, string>>;
} {
  run(kioskI18n);
  run(kioskExtras);
  const w = window as unknown as {
    KIOSK_LANGS: Array<{ code: string }>;
    KIOSK_TEXT: Record<string, Record<string, string>>;
  };
  return { langs: w.KIOSK_LANGS, text: w.KIOSK_TEXT };
}

/** What the server puts into the page as `window.XC_KIOSK_SCREEN` (`KioskScreen::not_set`, api/src/application/kiosk_screen.rs). */
export const PAGE_SCREEN = {
  quickAmounts: [100, 500, 1000, 5000],
  tiles: 6,
  idleSeconds: 120,
  reviewSeconds: 3,
  refreshSeconds: 10,
  popularDays: 30,
  set: false,
};

/** Opens `/kiosk<search>` with the page's body and scripts; `fetch` answers every request the page makes. */
export function openKiosk(search: string, fetch: KioskFetch, screen: object = PAGE_SCREEN): void {
  window.history.replaceState(null, '', '/kiosk' + search);
  (window as unknown as { XC_KIOSK_SCREEN: object }).XC_KIOSK_SCREEN = screen;
  const parsed = new DOMParser().parseFromString(kioskHtml, 'text/html');
  for (const script of parsed.querySelectorAll('script')) script.remove();
  document.body.className = parsed.body.className;
  document.body.innerHTML = parsed.body.innerHTML;
  globalThis.fetch = fetch as typeof globalThis.fetch;
  window.scrollTo = () => {};
  run(kioskI18n);
  run(kioskExtras);
  run(kioskJs);
}

export const el = <T extends HTMLElement = HTMLElement>(id: string): T => {
  const found = document.getElementById(id);
  if (!found) throw new Error(`#${id} missing`);
  return found as T;
};

const json = (body: unknown) =>
  new Response(JSON.stringify(body), { status: 200, headers: { 'content-type': 'application/json' } });

/** Our API as the kiosk reads it: two public branches, one rate and one payment method, the counter open. */
export function fakeApi(): KioskFetch {
  return async (url, init) => {
    const path = new URL(url, 'http://kiosk.test').pathname;
    if (path === '/api/branch-tablet/me') return json({ paired: false });
    if (path === '/api/public/host') return json({ org: 'XC' });
    if (path === '/api/public/orgs/XC/branches') {
      return json({
        branches: [
          { id: 'MAIN', code: 'XC-B', name: 'Main branch', address: null, openingHours: null, timezone: 'Asia/Bangkok', bookingEnabled: true, queueOpen: true },
          { id: 'B2', code: 'XC-2', name: 'Second branch', address: null, openingHours: null, timezone: 'Asia/Bangkok', bookingEnabled: true, queueOpen: true },
        ],
      });
    }
    if (path === '/api/kiosk') {
      const body = JSON.parse(String(init?.body ?? '{}')) as { action: string; payload: { branchId: string } };
      if (body.action === 'rates') {
        return json({
          branchId: body.payload.branchId,
          branchName: body.payload.branchId === 'MAIN' ? 'Main branch' : 'Second branch',
          open: true,
          rates: [{ code: 'USD', name: 'ดอลลาร์สหรัฐ', nameEn: 'US Dollar', countries: [], buy: 33.53, sell: 34.1 }],
          paymentMethods: [{ id: 'Cash', enabled: true, buy: true, sell: true, instruction: '' }],
          simulation: false,
          freshness: { stale: false, sourceDate: null, publishedAt: null },
        });
      }
    }
    return new Response('{}', { status: 404 });
  };
}

/** Waits until the page has drawn the rates of a branch (the currency tiles exist). */
export async function ratesDrawn(): Promise<void> {
  for (let i = 0; i < 100; i++) {
    if (document.querySelectorAll('#currency-shortcuts button').length > 0) return;
    await new Promise((r) => setTimeout(r, 10));
  }
  throw new Error('rates were never drawn');
}

/** Lets the page's pending fetches and their follow-ups settle. */
export const settle = () => new Promise((r) => setTimeout(r, 50));
