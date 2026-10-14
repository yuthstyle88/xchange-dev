import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th } from '../../../shared/i18n/th';
import { en } from '../../../shared/i18n/en';
import type { StaffFloorView } from '../../../shared/api/generated/staff_floor';
import type { StaffMeView } from '../../../shared/api/generated/staff_me';
import type { StaffCtx } from '../cards';
import { FloorCard } from './FloorCard';

function me(permissions = ['BranchDayOperate', 'CashTargetSet', 'AttendanceClock']): StaffMeView {
  return {
    userId: '00000000-0000-4000-8000-0000000000f1',
    name: 'Ploy',
    roles: [],
    branchId: 'MAIN',
    branchName: 'XC-B',
    permissions,
  };
}

function floor(over: Partial<StaffFloorView> = {}): StaffFloorView {
  return {
    phase: 'OPEN',
    hours: null,
    myClockIn: null,
    myShift: null,
    cutoff: null,
    target: { amounts: { THB: '1000', USD: '50' }, at: '2026-10-15T00:00:00Z' },
    safe: null,
    ...over,
  };
}

function makeCtx(over: Partial<StaffFloorView> = {}, perms?: string[]): StaffCtx {
  return {
    me: me(perms),
    floor: floor(over),
    refresh: vi.fn(async () => {}),
  } as unknown as StaffCtx;
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

describe('FloorCard: wordedError on failed calls (WE-40 #1789)', () => {
  let host: HTMLDivElement | undefined;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host?.remove();
    host = undefined;
    setLang('th');
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  async function flush(): Promise<void> {
    for (let i = 0; i < 10; i++) {
      await new Promise((r) => setTimeout(r, 10));
    }
  }

  // ── Catch 1: loadRates (mount) ─────────────────────────────────────────────
  describe('Catch 1: loadRates', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new TypeError('Failed to fetch')));

      const ctx = makeCtx();
      dispose = render(() => <FloorCard ctx={ctx} />, host!);
      await flush();

      const errEl = host!.querySelector('[data-testid="floor-error"]');
      expect(errEl, 'floor-error in TH').not.toBeNull();
      expect(errEl?.textContent).toBe(th['error.network']);

      setLang('en');
      host!.innerHTML = '';
      dispose?.();
      dispose = render(() => <FloorCard ctx={ctx} />, host!);
      await flush();

      const errElEn = host!.querySelector('[data-testid="floor-error"]');
      expect(errElEn, 'floor-error in EN').not.toBeNull();
      expect(errElEn?.textContent).toBe(en['error.network']);
    });

    it('(2) 500 refusal shows error.serverError in TH (even with keyed non-server code)', async () => {
      vi.stubGlobal(
        'fetch',
        vi.fn().mockResolvedValue(
          json(500, {
            error: {
              code: 'roleNotAllowedHere',
              message: 'internal server error',
            },
          }),
        ),
      );

      const ctx = makeCtx();
      dispose = render(() => <FloorCard ctx={ctx} />, host!);
      await flush();

      const errEl = host!.querySelector('[data-testid="floor-error"]');
      expect(errEl).not.toBeNull();
      expect(errEl?.textContent).toBe(th['error.serverError']);
    });

    it('(3) unkeyed code with server message shows message in TH', async () => {
      const serverSentence = 'ไม่สามารถอ่านเรทของสาขานี้ได้ในขณะนี้';
      vi.stubGlobal(
        'fetch',
        vi.fn().mockResolvedValue(
          json(400, {
            error: {
              code: 'unkeyedLoadRatesFailure',
              message: serverSentence,
            },
          }),
        ),
      );

      const ctx = makeCtx();
      dispose = render(() => <FloorCard ctx={ctx} />, host!);
      await flush();

      const errEl = host!.querySelector('[data-testid="floor-error"]');
      expect(errEl).not.toBeNull();
      expect(errEl?.textContent).toBe(serverSentence);
    });
  });

  // ── Catch 2: handleClock ───────────────────────────────────────────────────
  describe('Catch 2: handleClock', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_published_rates')) {
          return json(200, { version: 1, changed: false, data: null });
        }
        if (url.includes('/api/attendance')) {
          throw new TypeError('Failed to fetch');
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }));

      const ctx = makeCtx();
      dispose = render(() => <FloorCard ctx={ctx} />, host!);
      await flush();

      const btn = host!.querySelector<HTMLButtonElement>('[data-testid="floor-clock"]');
      btn?.click();
      await flush();

      const errEl = host!.querySelector('[data-testid="floor-error"]');
      expect(errEl, 'floor-error in TH').not.toBeNull();
      expect(errEl?.textContent).toBe(th['error.network']);

      setLang('en');
      btn?.click();
      await flush();

      const errElEn = host!.querySelector('[data-testid="floor-error"]');
      expect(errElEn, 'floor-error in EN').not.toBeNull();
      expect(errElEn?.textContent).toBe(en['error.network']);
    });

    it('(2) 500 refusal shows error.serverError in TH', async () => {
      vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_published_rates')) {
          return json(200, { version: 1, changed: false, data: null });
        }
        if (url.includes('/api/attendance')) {
          return json(500, {
            error: {
              code: 'roleNotAllowedHere',
              message: 'internal server error on clock',
            },
          });
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }));

      const ctx = makeCtx();
      dispose = render(() => <FloorCard ctx={ctx} />, host!);
      await flush();

      const btn = host!.querySelector<HTMLButtonElement>('[data-testid="floor-clock"]');
      btn?.click();
      await flush();

      const errEl = host!.querySelector('[data-testid="floor-error"]');
      expect(errEl).not.toBeNull();
      expect(errEl?.textContent).toBe(th['error.serverError']);
    });

    it('(3) unkeyed code with server message shows message in TH', async () => {
      const serverSentence = 'ไม่สามารถลงเวลาเข้าออกได้ในขณะนี้';
      vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_published_rates')) {
          return json(200, { version: 1, changed: false, data: null });
        }
        if (url.includes('/api/attendance')) {
          return json(400, {
            error: {
              code: 'unkeyedAttendanceFailure',
              message: serverSentence,
            },
          });
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }));

      const ctx = makeCtx();
      dispose = render(() => <FloorCard ctx={ctx} />, host!);
      await flush();

      const btn = host!.querySelector<HTMLButtonElement>('[data-testid="floor-clock"]');
      btn?.click();
      await flush();

      const errEl = host!.querySelector('[data-testid="floor-error"]');
      expect(errEl).not.toBeNull();
      expect(errEl?.textContent).toBe(serverSentence);
    });
  });

  // ── Catch 3: handleStartService rates reload ──────────────────────────────
  describe('Catch 3: handleStartService rates reload', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      let callCount = 0;
      vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_published_rates')) {
          callCount++;
          if (callCount === 1) {
            // initial mount succeeds with no publication
            return json(200, { version: 1, changed: false, data: null });
          }
          throw new TypeError('Failed to fetch');
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }));

      const ctx = makeCtx({ phase: 'READY' });
      dispose = render(() => <FloorCard ctx={ctx} />, host!);
      await flush();

      // Click start service
      const startBtn = host!.querySelector<HTMLButtonElement>('[data-testid="floor-start"]')!;
      startBtn.click();
      await flush();

      // Confirm dialog
      const dialog = host!.querySelector('dialog')!;
      const buttons = dialog.querySelectorAll('button');
      buttons[1]!.click();
      await flush();

      const errEl = host!.querySelector('[data-testid="floor-error"]');
      expect(errEl, 'floor-error in TH').not.toBeNull();
      expect(errEl?.textContent).toBe(th['error.network']);

      // In EN
      setLang('en');
      startBtn.click();
      await flush();
      const dialogEn = host!.querySelector('dialog')!;
      const buttonsEn = dialogEn.querySelectorAll('button');
      buttonsEn[1]!.click();
      await flush();

      const errElEn = host!.querySelector('[data-testid="floor-error"]');
      expect(errElEn, 'floor-error in EN').not.toBeNull();
      expect(errElEn?.textContent).toBe(en['error.network']);
    });

    it('(2) 500 refusal shows error.serverError in TH', async () => {
      let callCount = 0;
      vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_published_rates')) {
          callCount++;
          if (callCount === 1) {
            return json(200, { version: 1, changed: false, data: null });
          }
          return json(500, {
            error: {
              code: 'roleNotAllowedHere',
              message: 'failed rates in start service',
            },
          });
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }));

      const ctx = makeCtx({ phase: 'READY' });
      dispose = render(() => <FloorCard ctx={ctx} />, host!);
      await flush();

      const startBtn = host!.querySelector<HTMLButtonElement>('[data-testid="floor-start"]')!;
      startBtn.click();
      await flush();

      const dialog = host!.querySelector('dialog')!;
      const buttons = dialog.querySelectorAll('button');
      buttons[1]!.click();
      await flush();

      const errEl = host!.querySelector('[data-testid="floor-error"]');
      expect(errEl).not.toBeNull();
      expect(errEl?.textContent).toBe(th['error.serverError']);
    });

    it('(3) unkeyed code with server message shows message in TH', async () => {
      const serverSentence = 'ไม่สามารถอ่านเรทเพื่อเริ่มกะได้';
      let callCount = 0;
      vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_published_rates')) {
          callCount++;
          if (callCount === 1) {
            return json(200, { version: 1, changed: false, data: null });
          }
          return json(400, {
            error: {
              code: 'unkeyedRatesInStartFailure',
              message: serverSentence,
            },
          });
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }));

      const ctx = makeCtx({ phase: 'READY' });
      dispose = render(() => <FloorCard ctx={ctx} />, host!);
      await flush();

      const startBtn = host!.querySelector<HTMLButtonElement>('[data-testid="floor-start"]')!;
      startBtn.click();
      await flush();

      const dialog = host!.querySelector('dialog')!;
      const buttons = dialog.querySelectorAll('button');
      buttons[1]!.click();
      await flush();

      const errEl = host!.querySelector('[data-testid="floor-error"]');
      expect(errEl).not.toBeNull();
      expect(errEl?.textContent).toBe(serverSentence);
    });
  });

  // ── Catch 4: handleStartService startCmd ───────────────────────────────────
  describe('Catch 4: handleStartService startCmd', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_published_rates')) {
          return json(200, {
            version: 1,
            changed: false,
            data: { publication: { publishedAt: '2026-10-15T00:00:00Z' } },
          });
        }
        if (url.includes('/api/branch-day/start')) {
          throw new TypeError('Failed to fetch');
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }));

      const ctx = makeCtx({ phase: 'READY' });
      dispose = render(() => <FloorCard ctx={ctx} />, host!);
      await flush();

      const startBtn = host!.querySelector<HTMLButtonElement>('[data-testid="floor-start"]')!;
      startBtn.click();
      await flush();

      const dialog = host!.querySelector('dialog')!;
      const buttons = dialog.querySelectorAll('button');
      buttons[1]!.click();
      await flush();

      const errEl = host!.querySelector('[data-testid="floor-error"]');
      expect(errEl, 'floor-error in TH').not.toBeNull();
      expect(errEl?.textContent).toBe(th['error.network']);

      setLang('en');
      startBtn.click();
      await flush();
      const dialogEn = host!.querySelector('dialog')!;
      const buttonsEn = dialogEn.querySelectorAll('button');
      buttonsEn[1]!.click();
      await flush();

      const errElEn = host!.querySelector('[data-testid="floor-error"]');
      expect(errElEn, 'floor-error in EN').not.toBeNull();
      expect(errElEn?.textContent).toBe(en['error.network']);
    });

    it('(2) 500 refusal shows error.serverError in TH', async () => {
      vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_published_rates')) {
          return json(200, {
            version: 1,
            changed: false,
            data: { publication: { publishedAt: '2026-10-15T00:00:00Z' } },
          });
        }
        if (url.includes('/api/branch-day/start')) {
          return json(500, {
            error: {
              code: 'roleNotAllowedHere',
              message: 'start service 500 failure',
            },
          });
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }));

      const ctx = makeCtx({ phase: 'READY' });
      dispose = render(() => <FloorCard ctx={ctx} />, host!);
      await flush();

      const startBtn = host!.querySelector<HTMLButtonElement>('[data-testid="floor-start"]')!;
      startBtn.click();
      await flush();

      const dialog = host!.querySelector('dialog')!;
      const buttons = dialog.querySelectorAll('button');
      buttons[1]!.click();
      await flush();

      const errEl = host!.querySelector('[data-testid="floor-error"]');
      expect(errEl).not.toBeNull();
      expect(errEl?.textContent).toBe(th['error.serverError']);
    });

    it('(3) unkeyed code with server message shows message in TH', async () => {
      const serverSentence = 'ไม่สามารถเริ่มเปิดบริการสาขาได้';
      vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_published_rates')) {
          return json(200, {
            version: 1,
            changed: false,
            data: { publication: { publishedAt: '2026-10-15T00:00:00Z' } },
          });
        }
        if (url.includes('/api/branch-day/start')) {
          return json(400, {
            error: {
              code: 'unkeyedStartDayFailure',
              message: serverSentence,
            },
          });
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }));

      const ctx = makeCtx({ phase: 'READY' });
      dispose = render(() => <FloorCard ctx={ctx} />, host!);
      await flush();

      const startBtn = host!.querySelector<HTMLButtonElement>('[data-testid="floor-start"]')!;
      startBtn.click();
      await flush();

      const dialog = host!.querySelector('dialog')!;
      const buttons = dialog.querySelectorAll('button');
      buttons[1]!.click();
      await flush();

      const errEl = host!.querySelector('[data-testid="floor-error"]');
      expect(errEl).not.toBeNull();
      expect(errEl?.textContent).toBe(serverSentence);
    });
  });

  // ── Catch 5: handleCutoff ──────────────────────────────────────────────────
  describe('Catch 5: handleCutoff', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_published_rates')) {
          return json(200, { version: 1, changed: false, data: null });
        }
        if (url.includes('/api/branch-day/cutoff')) {
          throw new TypeError('Failed to fetch');
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }));

      const ctx = makeCtx({ phase: 'OPEN' });
      dispose = render(() => <FloorCard ctx={ctx} />, host!);
      await flush();

      const btn = host!.querySelector<HTMLButtonElement>('[data-testid="floor-cutoff"]');
      btn?.click();
      await flush();

      const errEl = host!.querySelector('[data-testid="floor-error"]');
      expect(errEl, 'floor-error in TH').not.toBeNull();
      expect(errEl?.textContent).toBe(th['error.network']);

      setLang('en');
      btn?.click();
      await flush();

      const errElEn = host!.querySelector('[data-testid="floor-error"]');
      expect(errElEn, 'floor-error in EN').not.toBeNull();
      expect(errElEn?.textContent).toBe(en['error.network']);
    });

    it('(2) 500 refusal shows error.serverError in TH', async () => {
      vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_published_rates')) {
          return json(200, { version: 1, changed: false, data: null });
        }
        if (url.includes('/api/branch-day/cutoff')) {
          return json(500, {
            error: {
              code: 'roleNotAllowedHere',
              message: 'cutoff internal error',
            },
          });
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }));

      const ctx = makeCtx({ phase: 'OPEN' });
      dispose = render(() => <FloorCard ctx={ctx} />, host!);
      await flush();

      const btn = host!.querySelector<HTMLButtonElement>('[data-testid="floor-cutoff"]');
      btn?.click();
      await flush();

      const errEl = host!.querySelector('[data-testid="floor-error"]');
      expect(errEl).not.toBeNull();
      expect(errEl?.textContent).toBe(th['error.serverError']);
    });

    it('(3) unkeyed code with server message shows message in TH', async () => {
      const serverSentence = 'ไม่สามารถทำการตัดยอดได้';
      vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_published_rates')) {
          return json(200, { version: 1, changed: false, data: null });
        }
        if (url.includes('/api/branch-day/cutoff')) {
          return json(400, {
            error: {
              code: 'unkeyedCutoffFailure',
              message: serverSentence,
            },
          });
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }));

      const ctx = makeCtx({ phase: 'OPEN' });
      dispose = render(() => <FloorCard ctx={ctx} />, host!);
      await flush();

      const btn = host!.querySelector<HTMLButtonElement>('[data-testid="floor-cutoff"]');
      btn?.click();
      await flush();

      const errEl = host!.querySelector('[data-testid="floor-error"]');
      expect(errEl).not.toBeNull();
      expect(errEl?.textContent).toBe(serverSentence);
    });
  });

  // ── Catch 6: handleTargetSubmit ───────────────────────────────────────────
  describe('Catch 6: handleTargetSubmit', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_published_rates')) {
          return json(200, { version: 1, changed: false, data: null });
        }
        if (url.includes('/api/branch-day/target')) {
          throw new TypeError('Failed to fetch');
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }));

      const ctx = makeCtx();
      dispose = render(() => <FloorCard ctx={ctx} />, host!);
      await flush();

      const form = host!.querySelector<HTMLFormElement>('[data-testid="floor-target-form"]')!;
      form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
      await flush();

      const errEl = host!.querySelector('[data-testid="floor-error"]');
      expect(errEl, 'floor-error in TH').not.toBeNull();
      expect(errEl?.textContent).toBe(th['error.network']);

      setLang('en');
      form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
      await flush();

      const errElEn = host!.querySelector('[data-testid="floor-error"]');
      expect(errElEn, 'floor-error in EN').not.toBeNull();
      expect(errElEn?.textContent).toBe(en['error.network']);
    });

    it('(2) 500 refusal shows error.serverError in TH', async () => {
      vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_published_rates')) {
          return json(200, { version: 1, changed: false, data: null });
        }
        if (url.includes('/api/branch-day/target')) {
          return json(500, {
            error: {
              code: 'roleNotAllowedHere',
              message: 'target 500 failure',
            },
          });
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }));

      const ctx = makeCtx();
      dispose = render(() => <FloorCard ctx={ctx} />, host!);
      await flush();

      const form = host!.querySelector<HTMLFormElement>('[data-testid="floor-target-form"]')!;
      form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
      await flush();

      const errEl = host!.querySelector('[data-testid="floor-error"]');
      expect(errEl).not.toBeNull();
      expect(errEl?.textContent).toBe(th['error.serverError']);
    });

    it('(3) unkeyed code with server message shows message in TH', async () => {
      const serverSentence = 'ไม่สามารถบันทึกเป้าหมายเงินสดได้';
      vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_published_rates')) {
          return json(200, { version: 1, changed: false, data: null });
        }
        if (url.includes('/api/branch-day/target')) {
          return json(400, {
            error: {
              code: 'unkeyedTargetFailure',
              message: serverSentence,
            },
          });
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }));

      const ctx = makeCtx();
      dispose = render(() => <FloorCard ctx={ctx} />, host!);
      await flush();

      const form = host!.querySelector<HTMLFormElement>('[data-testid="floor-target-form"]')!;
      form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
      await flush();

      const errEl = host!.querySelector('[data-testid="floor-error"]');
      expect(errEl).not.toBeNull();
      expect(errEl?.textContent).toBe(serverSentence);
    });
  });
});
