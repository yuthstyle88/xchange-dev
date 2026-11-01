// FE-T11 #5858 (docs/design/fe-triage-1101.md §3 W7): the CLOSED day phase on /staff reads "ปิดวันทำการแล้ว" /
// "Day closed". "ปิดกะ" is the shift close, not the day. Written before the implementation by the ticket author
// claude/01TxaRJG. Do not edit, delete or weaken; add tests in a file of your own.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { makeStaffFloorView, makeStaffMeView } from '../../../shared/testing/fixtures';
import type { StaffCtx } from '../cards';
import { FloorCard } from './FloorCard';

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function ctx(): StaffCtx {
  return {
    me: makeStaffMeView({
      userId: '00000000-0000-4000-8000-0000000000f1',
      name: 'Ploy',
      roles: [],
      branchId: 'MAIN',
      branchName: 'XC-B',
      permissions: [],
    }),
    floor: makeStaffFloorView({
      phase: 'CLOSED',
      hours: null,
      myClockIn: null,
      myShift: null,
      cutoff: null,
      target: null,
      safe: null,
    }),
    refresh: vi.fn(async () => {}),
  };
}

async function flush(): Promise<void> {
  for (let i = 0; i < 6; i += 1) await new Promise((r) => setTimeout(r, 0));
}

const phase = () => host.querySelector('[data-testid="floor-phase"]')?.textContent?.trim();

beforeEach(() => {
  vi.stubGlobal(
    'fetch',
    vi.fn(async () =>
      new Response(JSON.stringify({ error: { code: 'notFound', message: 'x' } }), {
        status: 404,
        headers: { 'content-type': 'application/json' },
      }),
    ),
  );
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.unstubAllGlobals();
  setLang('th');
});

describe('FE-T11 #5858 W7 day phase CLOSED label', () => {
  it('C1 th: the floor phase reads "ปิดวันทำการแล้ว", not the shift word "ปิดกะ"', async () => {
    setLang('th');
    dispose = render(() => <FloorCard ctx={ctx()} />, host);
    await flush();
    expect(phase()).toBe('ปิดวันทำการแล้ว');
  });

  it('C2 en: the floor phase reads "Day closed"', async () => {
    setLang('en');
    dispose = render(() => <FloorCard ctx={ctx()} />, host);
    await flush();
    expect(phase()).toBe('Day closed');
  });
});
