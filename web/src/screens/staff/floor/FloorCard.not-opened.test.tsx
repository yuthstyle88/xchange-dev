// F36b #6343: /staff shows "ยังไม่เปิดวันทำการ" / "Not opened yet" when phase is NOT_OPENED.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { makeStaffFloorView, makeStaffMeView } from '../../../shared/testing/fixtures';
import type { StaffCtx } from '../cards';
import { FloorCard } from './FloorCard';

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function ctx(phase: string): StaffCtx {
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
      phase,
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

describe('F36b #6343 day phase NOT_OPENED label', () => {
  it('phase NOT_OPENED shows "ยังไม่เปิดวันทำการ" in Thai', async () => {
    setLang('th');
    dispose = render(() => <FloorCard ctx={ctx('NOT_OPENED')} />, host);
    await flush();
    expect(phase()).toBe('ยังไม่เปิดวันทำการ');
  });

  it('phase NOT_OPENED shows "Not opened yet" in English', async () => {
    setLang('en');
    dispose = render(() => <FloorCard ctx={ctx('NOT_OPENED')} />, host);
    await flush();
    expect(phase()).toBe('Not opened yet');
  });

  it('phase CLOSED still shows "ปิดวันทำการแล้ว"', async () => {
    setLang('th');
    dispose = render(() => <FloorCard ctx={ctx('CLOSED')} />, host);
    await flush();
    expect(phase()).toBe('ปิดวันทำการแล้ว');
  });
});
