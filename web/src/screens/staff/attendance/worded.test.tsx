import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th } from '../../../shared/i18n/th';
import { en } from '../../../shared/i18n/en';
import type { StaffFloorView } from '../../../shared/api/generated/staff_floor';
import type { StaffMeView } from '../../../shared/api/generated/staff_me';
import type { StaffCtx } from '../cards';
import { AttendanceCard } from './AttendanceCard';

function me(): StaffMeView {
  return {
    userId: '00000000-0000-4000-8000-0000000000d1',
    name: 'Ploy',
    roles: [],
    branchId: 'MAIN',
    branchName: 'XC-B',
    permissions: ['AttendanceClock'],
  };
}

function floor(): StaffFloorView {
  return {
    phase: 'OPEN',
    hours: null,
    myClockIn: null,
    myShift: null,
    cutoff: null,
    target: null,
    safe: null,
  };
}

function makeCtx(): StaffCtx {
  return {
    me: me(),
    floor: floor(),
    refresh: vi.fn(async () => {}),
  } as unknown as StaffCtx;
}

describe('AttendanceCard: wordedError on failed calls (WE-39 #1788)', () => {
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

  it('(1) a network TypeError shows error.network in TH and EN', async () => {
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new TypeError('Failed to fetch')));

    const ctx = makeCtx();
    dispose = render(() => <AttendanceCard ctx={ctx} />, host!);
    await flush();

    const errEl = host!.querySelector('[data-testid="attendance-error"]');
    expect(errEl, 'attendance-error must be rendered on network error').not.toBeNull();
    expect(errEl?.textContent).toBe(th['error.network']);

    // Switch to EN
    setLang('en');
    host!.innerHTML = '';
    dispose?.();
    dispose = render(() => <AttendanceCard ctx={ctx} />, host!);
    await flush();

    const errElEn = host!.querySelector('[data-testid="attendance-error"]');
    expect(errElEn, 'attendance-error in EN').not.toBeNull();
    expect(errElEn?.textContent).toBe(en['error.network']);
  });

  it('(2) a 500 refusal shows error.serverError in TH (even with keyed non-server code)', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(
        new Response(
          JSON.stringify({
            error: {
              code: 'roleNotAllowedHere',
              message: 'internal error occurred on server',
            },
          }),
          { status: 500, headers: { 'content-type': 'application/json' } }
        )
      )
    );

    const ctx = makeCtx();
    dispose = render(() => <AttendanceCard ctx={ctx} />, host!);
    await flush();

    const errEl = host!.querySelector('[data-testid="attendance-error"]');
    expect(errEl, 'attendance-error must be rendered on 500 error').not.toBeNull();
    expect(errEl?.textContent).toBe(th['error.serverError']);
  });

  it('(3) an unknown code with a server message shows that message', async () => {
    const serverSentence = 'ไม่สามารถอ่านประวัติการลงเวลาได้ขณะนี้';
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(
        new Response(
          JSON.stringify({
            error: {
              code: 'unknownAttendanceRefusal',
              message: serverSentence,
            },
          }),
          { status: 400, headers: { 'content-type': 'application/json' } }
        )
      )
    );

    const ctx = makeCtx();
    dispose = render(() => <AttendanceCard ctx={ctx} />, host!);
    await flush();

    const errEl = host!.querySelector('[data-testid="attendance-error"]');
    expect(errEl, 'attendance-error must be rendered on unknown code refusal').not.toBeNull();
    expect(errEl?.textContent).toBe(serverSentence);
  });
});
