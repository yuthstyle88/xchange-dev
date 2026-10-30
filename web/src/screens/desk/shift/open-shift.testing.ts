// #5641 (DESK-SHIFT T4, docs/design/desk-shift-open.md §3 T4): /desk offers its exchange steps only once the caller's
// shift is open. Test support, written by the ticket author claude/studio-claude-15; do not edit. `withOpenShift`
// wraps a test's fake `fetch` so the tests written before #5641 keep their meaning: the caller is a clocked-in teller
// with an open shift. It adds `ShiftOperate` to the session `me` the fake answers, answers `staff_me`, `staff_floor`
// (clocked in, shift open) and `cash_counts` (none waiting) itself, and passes every other request to the fake.
// The `me` it hands on is a whole `Me`: `roles` and `branches` default to [] when a fake left them out, because the
// page now reacts to `me` and `AccountBar` reads `me.roles` (a throw there aborts the update the session poll runs).
import { makeStaffFloorView, makeStaffMeView } from '../../../shared/testing/fixtures';

type FetchFn = (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;

function urlOf(input: RequestInfo | URL): URL {
  const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
  return new URL(raw, 'http://localhost');
}

function view(data: unknown): Response {
  return new Response(JSON.stringify({ version: 1, changed: true, data }), {
    status: 200,
    headers: { 'content-type': 'application/json' },
  });
}

export function withOpenShift(inner: FetchFn): FetchFn {
  return async (input, init) => {
    const u = urlOf(input);
    const method = (init?.method ?? 'GET').toUpperCase();
    if (method === 'GET' && u.pathname === '/api/v3/views/staff_me') {
      return view(
        makeStaffMeView({
          userId: 'u-1',
          name: 'Ploy',
          roles: [],
          branchId: 'MAIN',
          branchName: 'XC-B',
          permissions: ['AttendanceClock', 'ShiftOperate'],
        }),
      );
    }
    if (method === 'GET' && u.pathname === '/api/v3/views/staff_floor') {
      return view(
        makeStaffFloorView({
          phase: 'OPEN',
          hours: null,
          myClockIn: '2026-10-30T01:00:00Z',
          myShift: { id: 'SH-open', openedAt: '2026-10-30T01:05:00Z' },
          cutoff: null,
          target: null,
          safe: null,
        }),
      );
    }
    if (method === 'GET' && u.pathname === '/api/v3/views/cash_counts') return view([]);
    const res = await inner(input, init);
    if (method === 'GET' && u.pathname === '/api/v3/views/me' && res.ok) {
      try {
        const body = (await res.clone().json()) as { data?: { permissions?: string[]; roles?: unknown[]; branches?: unknown[] } };
        if (body?.data && typeof body.data === 'object') {
          const perms = Array.isArray(body.data.permissions) ? body.data.permissions : [];
          if (!perms.includes('ShiftOperate')) body.data.permissions = [...perms, 'ShiftOperate'];
          if (!Array.isArray(body.data.roles)) body.data.roles = [];
          if (!Array.isArray(body.data.branches)) body.data.branches = [];
          return new Response(JSON.stringify(body), { status: res.status, headers: res.headers });
        }
      } catch {
        // not JSON: pass it on as it came
      }
    }
    return res;
  };
}
