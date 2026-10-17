// #2151: Desk rate step shows wordedError(e) when the desk settings fail to load, instead of "no shift is open".
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../../shared/api/client';
import { errorText, setLang, t } from '../../../../shared/i18n';
import { OWNER_DESK, STAFF_DESK, type DeskPath } from '../../desk-path';
import { createDeskStore, type DeskStore } from '../../desk-store';
import * as rateTexts from './i18n';
import { step } from './step';

const ME = () => ({
  id: 'u-1',
  name: 'Ploy',
  role: 'TELLER',
  branch: 'MAIN',
  permissions: [],
});

const NO_SHIFT_TH = rateTexts.th.noShift;
const NEED_SHIFT_AND_RATE_TH = rateTexts.th.needShiftAndRate;

type Answer = { status: number; body?: unknown; raw?: string };

function fakeApi(configReply: () => Answer) {
  const fetchFn = vi.fn(async (input: RequestInfo | URL) => {
    const path = String(input).split('?')[0];
    if (path === `/api/v3/views/${OWNER_DESK.configView}` || path === `/api/v3/views/${STAFF_DESK.configView}`) {
      const a = configReply();
      const text = a.raw ?? JSON.stringify(a.body ?? {});
      return new Response(text, {
        status: a.status,
        headers: { 'content-type': a.raw ? 'text/html' : 'application/json' },
      });
    }
    return new Response(JSON.stringify({ error: { code: 'notFound', message: 'not found' } }), {
      status: 404,
      headers: { 'content-type': 'application/json' },
    });
  });
  return new ApiClient(fetchFn as unknown as typeof fetch);
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  setLang('th');
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.restoreAllMocks();
  setLang('th');
});

function mountRateStep(path: DeskPath, configReply: () => Answer): DeskStore {
  const api = fakeApi(configReply);
  let store!: DeskStore;
  dispose = render(() => {
    store = createDeskStore({ api, path, me: ME as any });
    return step.component({ store });
  }, host);
  return store;
}

const notice = () => host.querySelector<HTMLElement>('.desk-notice');
const alertEl = () => host.querySelector<HTMLElement>('[role="alert"]');
const submitBtn = () => host.querySelector<HTMLButtonElement>('button[type="submit"]');

for (const [which, path] of [
  ['OWNER_DESK', OWNER_DESK],
  ['STAFF_DESK', STAFF_DESK],
] as const) {
  describe(`#2151 desk rate step configError on ${which}`, () => {
    it('row 1: when the config read fails with a 5xx, the rate step shows the worded server-error text (not "no shift") and submit refuses with it', async () => {
      const store = mountRateStep(path, () => ({
        status: 502,
        raw: '<html>502 Bad Gateway</html>',
      }));

      const expected = t('error.serverError');
      await vi.waitFor(() => {
        expect(store.configError()).toBe(expected);
      });

      expect(notice()?.textContent).toBe(expected);
      expect(host.textContent).not.toContain(NO_SHIFT_TH);
      expect(host.textContent).not.toContain('502 Bad Gateway');

      submitBtn()!.click();
      await vi.waitFor(() => {
        expect(alertEl()?.textContent).toBe(expected);
      });
      expect(host.textContent).not.toContain(NEED_SHIFT_AND_RATE_TH);
      expect(store.step()).toBe(0);
    });

    it('row 2: when the config read is refused with a code (roleNotAllowedHere), the rate step shows the worded text for that code in the current language', async () => {
      setLang('th');
      const expectedTh = errorText('roleNotAllowedHere', 'raw server refusal text');
      const storeTh = mountRateStep(path, () => ({
        status: 403,
        body: { error: { code: 'roleNotAllowedHere', message: 'raw server refusal text' } },
      }));

      await vi.waitFor(() => {
        expect(storeTh.configError()).toBe(expectedTh);
      });
      expect(notice()?.textContent).toBe(expectedTh);
      expect(host.textContent).not.toContain(NO_SHIFT_TH);
      expect(host.textContent).not.toContain('raw server refusal text');

      submitBtn()!.click();
      await vi.waitFor(() => {
        expect(alertEl()?.textContent).toBe(expectedTh);
      });
      expect(host.textContent).not.toContain(NEED_SHIFT_AND_RATE_TH);

      dispose?.();
      dispose = undefined;
      host.innerHTML = '';

      setLang('en');
      const expectedEn = errorText('roleNotAllowedHere', 'raw server refusal text');
      expect(expectedEn).not.toBe(expectedTh);
      const storeEn = mountRateStep(path, () => ({
        status: 403,
        body: { error: { code: 'roleNotAllowedHere', message: 'raw server refusal text' } },
      }));

      await vi.waitFor(() => {
        expect(storeEn.configError()).toBe(expectedEn);
      });
      expect(notice()?.textContent).toBe(expectedEn);
      expect(host.textContent).not.toContain(rateTexts.en.noShift);
      expect(host.textContent).not.toContain('raw server refusal text');

      submitBtn()!.click();
      await vi.waitFor(() => {
        expect(alertEl()?.textContent).toBe(expectedEn);
      });
      expect(host.textContent).not.toContain(rateTexts.en.needShiftAndRate);
    });

    it('row 3: when the config loads and the shift is closed, the rate step shows "no shift" and submit refuses with needShiftAndRate as today', async () => {
      const store = mountRateStep(path, () => ({
        status: 200,
        body: { version: 1, changed: true, data: { shiftOpen: false, rates: [] } },
      }));

      await vi.waitFor(() => {
        expect(store.config()?.shiftOpen).toBe(false);
      });
      expect(store.configError()).toBeUndefined();
      expect(notice()?.textContent).toBe(NO_SHIFT_TH);

      submitBtn()!.click();
      await vi.waitFor(() => {
        expect(alertEl()?.textContent).toBe(NEED_SHIFT_AND_RATE_TH);
      });
      expect(store.step()).toBe(0);
    });

    it('row 4: when the config loads and the shift is open, no notice is shown as today', async () => {
      const store = mountRateStep(path, () => ({
        status: 200,
        body: {
          version: 1,
          changed: true,
          data: { shiftOpen: true, rates: [{ code: 'USD', buyRate: '33.50', sellRate: '34.00', unitSize: '1' }] },
        },
      }));

      await vi.waitFor(() => {
        expect(store.config()?.shiftOpen).toBe(true);
      });
      expect(store.configError()).toBeUndefined();
      expect(notice()).toBeNull();
      expect(host.textContent).not.toContain(NO_SHIFT_TH);
    });
  });
}
