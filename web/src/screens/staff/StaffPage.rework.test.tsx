import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { errorText, screenT, setLang } from '../../shared/i18n';
import { ApiClient } from '../../shared/api/client';
import type { StaffMeView } from '../../shared/api/generated/staff_me';
import type { StaffFloorView } from '../../shared/api/generated/staff_floor';
import { th, en } from './i18n';

type PageModule = {
  StaffPage: (p: {
    api: ApiClient;
    cards: readonly unknown[];
    navigate: (url: string) => void;
  }) => JSX.Element;
};

const pageMods = import.meta.glob<PageModule>('./StaffPage.tsx');

function staffMe(permissions: string[] = []): StaffMeView {
  return {
    userId: '00000000-0000-4000-8000-0000000000a1',
    name: 'Ploy',
    roles: [],
    branchId: 'MAIN',
    branchName: 'XC-B',
    permissions,
  };
}

const FLOOR_WITH_HOURS: StaffFloorView = {
  phase: 'OPEN',
  hours: { open: '2026-10-11T03:00:00Z', close: '2026-10-11T13:00:00Z' },
  myClockIn: null,
  myShift: null,
  cutoff: null,
  target: null,
  safe: null,
};

type Route = { status: number; body: unknown };
let routes: Record<string, () => Route>;
let fetchSpy: ReturnType<typeof vi.fn>;
let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function viewsOf(meAnswer: Route, floorAnswer: Route) {
  routes = {
    '/api/v3/views/staff_me': () => meAnswer,
    '/api/v3/views/staff_floor': () => floorAnswer,
  };
}

function ok(data: unknown): Route {
  return { status: 200, body: { version: 1, changed: true, data } };
}

async function mount() {
  const mod = await pageMods['./StaffPage.tsx']!();
  const navigate = vi.fn();
  const api = new ApiClient(fetchSpy as unknown as typeof fetch);
  dispose = render(() => <mod.StaffPage api={api} cards={[]} navigate={navigate} />, host);
  return navigate;
}

beforeEach(() => {
  setLang('th');
  host = document.createElement('div');
  document.body.appendChild(host);
  fetchSpy = vi.fn(async (input: RequestInfo | URL) => {
    const path = String(input).split('?')[0];
    const route = routes[path];
    if (!route) throw new Error(`unexpected request ${String(input)}`);
    const { status, body } = route();
    return new Response(JSON.stringify(body), {
      status,
      headers: { 'content-type': 'application/json' },
    });
  });
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.useRealTimers();
  setLang('th');
});

describe('StaffPage #1703: worded error when staff_floor read fails', () => {
  it('a 500 server failure on staff_floor shows a worded error alert, never drops silently', async () => {
    viewsOf(
      ok(staffMe()),
      { status: 500, body: { error: { code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง' } } },
    );
    await mount();

    await vi.waitFor(() => {
      const alert = host.querySelector('[role="alert"]');
      expect(alert, 'an element with role="alert" must be displayed').not.toBeNull();
      expect(alert!.textContent).toContain(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
    });
  });

  it('a refusal with a known code shows the worded refusal message from i18n, never raw code or message', async () => {
    viewsOf(
      ok(staffMe()),
      { status: 403, body: { error: { code: 'roleNotAllowedHere', message: 'raw.refused.string' } } },
    );
    await mount();

    await vi.waitFor(() => {
      const alert = host.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toContain(errorText('roleNotAllowedHere', 'refused'));
      expect(alert!.textContent).not.toContain('raw.refused.string');
      expect(alert!.textContent).not.toContain('roleNotAllowedHere');
    });
  });

  it('a refusal with an unknown code shows the screen generic worded error, never raw code or err.message', async () => {
    const t = screenT('staff', { th, en });
    viewsOf(
      ok(staffMe()),
      { status: 400, body: { error: { code: 'unknownCustomCode', message: 'internal database timeout details' } } },
    );
    await mount();

    await vi.waitFor(() => {
      const alert = host.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toBe(t('floorError'));
      expect(alert!.textContent).not.toContain('unknownCustomCode');
      expect(alert!.textContent).not.toContain('internal database timeout details');
    });
  });

  it('stale data from previous successful read remains visible, but the error is shown on failed refresh', async () => {
    let floorCall = 0;
    routes = {
      '/api/v3/views/staff_me': () => ok(staffMe()),
      '/api/v3/views/staff_floor': () => {
        floorCall++;
        if (floorCall === 1) {
          return ok(FLOOR_WITH_HOURS);
        }
        return { status: 500, body: { error: { code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง' } } };
      },
    };
    await mount();

    // First load succeeds: hours are displayed, no error alert
    await vi.waitFor(() => {
      const hours = host.querySelector('[data-testid="staff-hours"]');
      expect(hours?.textContent).toContain('10:00');
    });
    expect(host.querySelector('[role="alert"]')).toBeNull();

    // Click refresh: staff_floor fails with 500
    const refreshBtn = host.querySelector('[data-testid="staff-refresh"]') as HTMLButtonElement;
    expect(refreshBtn).not.toBeNull();
    refreshBtn.click();

    // Stale hours still visible, but error alert now appears
    await vi.waitFor(() => {
      const alert = host.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toContain(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
      const hours = host.querySelector('[data-testid="staff-hours"]');
      expect(hours?.textContent).toContain('10:00');
    });
  });

  it('shows worded error in English when language is set to English', async () => {
    setLang('en');
    const tEn = screenT('staff', { th, en });
    viewsOf(
      ok(staffMe()),
      { status: 400, body: { error: { code: 'unknownCustomCode', message: 'server internal detail' } } },
    );
    await mount();

    await vi.waitFor(() => {
      const alert = host.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toBe(tEn('floorError'));
      expect(alert!.textContent).not.toContain('server internal detail');
    });
  });
});
