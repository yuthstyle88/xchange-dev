import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../shared/i18n';
import { th } from '../../shared/i18n/th';
import { ApiClient } from '../../shared/api/client';
import { makeStaffFloorHours, makeStaffFloorView } from '../../shared/testing/fixtures';
import { StaffPage } from './StaffPage';

const FLOOR_WITH_HOURS = makeStaffFloorView({
  phase: 'OPEN',
  hours: makeStaffFloorHours({ open: '2026-10-11T03:00:00Z', close: '2026-10-11T13:00:00Z' }),
  myClockIn: null,
  myShift: null,
  cutoff: null,
  target: null,
  safe: null,
});

type Route = { status: number; body: unknown };
let routes: Record<string, () => Route>;
let fetchSpy: ReturnType<typeof vi.fn>;
let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function viewsOf(meAnswer: Route) {
  routes = {
    '/api/v3/views/staff_me': () => meAnswer,
    '/api/v3/views/staff_floor': () => ({
      status: 200,
      body: { version: 1, changed: true, data: FLOOR_WITH_HOURS },
    }),
  };
}

function mount() {
  const navigate = vi.fn();
  const api = new ApiClient(fetchSpy as unknown as typeof fetch);
  dispose = render(() => <StaffPage api={api} cards={[]} navigate={navigate} />, host);
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
  setLang('th');
});

describe('StaffPage #4305: 401 employeeAuthRequired vs signIn on staff_me', () => {
  it('staff_me -> 401 employeeAuthRequired never calls navigate and renders the worded refusal in th', async () => {
    viewsOf({
      status: 401,
      body: { error: { code: 'employeeAuthRequired', message: 'employee authentication required' } },
    });
    const navigate = mount();

    await vi.waitFor(() => {
      const alert = host.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toBe(th['error.employeeAuthRequired']);
    });
    expect(navigate).not.toHaveBeenCalled();
  });

  it('staff_me -> 401 signIn still calls navigate("/login?next=%2Fstaff") and renders no alert', async () => {
    viewsOf({
      status: 401,
      body: { error: { code: 'signIn', message: 'กรุณาเข้าสู่ระบบ' } },
    });
    const navigate = mount();

    await vi.waitFor(() => {
      expect(navigate).toHaveBeenCalledWith('/login?next=%2Fstaff');
    });
    expect(host.querySelector('[role="alert"]')).toBeNull();
  });
});
