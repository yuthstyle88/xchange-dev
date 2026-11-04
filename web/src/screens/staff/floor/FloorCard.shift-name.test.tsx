// CS-18 #6208 (docs/design/configurable-shifts.md §2.9): staff floor shows my shift name, business date and on-break.
// Behaviour rows F3–F5.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { makeStaffFloorMyShift, makeStaffFloorView, makeStaffMeView, type StaffFloorView } from '../../../shared/testing/fixtures';
import type { StaffCtx } from '../cards';
import { FloorCard } from './FloorCard';

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function ctx(floorOver: Partial<StaffFloorView> = {}): StaffCtx {
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
      phase: 'OPEN',
      hours: null,
      myClockIn: null,
      myShift: null,
      cutoff: null,
      target: null,
      safe: null,
      ...floorOver,
    }),
    refresh: vi.fn(async () => {}),
  };
}

async function flush(): Promise<void> {
  for (let i = 0; i < 6; i += 1) await new Promise((r) => setTimeout(r, 0));
}

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

describe('CS-18 #6208: floor card my shift line and on-break badge (F3–F5)', () => {
  describe('F3: shiftName and businessDate show on the my shift line', () => {
    it('th: shows "กะ C · 4/11" when shiftName is "C" and businessDate is "2026-11-04"', async () => {
      setLang('th');
      const shift = makeStaffFloorMyShift({
        shiftName: 'C',
        businessDate: '2026-11-04',
      });
      dispose = render(() => <FloorCard ctx={ctx({ myShift: shift })} />, host);
      await flush();

      const line = host.querySelector('[data-testid="floor-my-shift"]');
      expect(line, 'floor my-shift line exists').not.toBeNull();
      expect(line?.textContent).toContain('กะ C · 4/11');
    });

    it('en: shows "Shift C · 4/11" when shiftName is "C" and businessDate is "2026-11-04"', async () => {
      setLang('en');
      const shift = makeStaffFloorMyShift({
        shiftName: 'C',
        businessDate: '2026-11-04',
      });
      dispose = render(() => <FloorCard ctx={ctx({ myShift: shift })} />, host);
      await flush();

      const line = host.querySelector('[data-testid="floor-my-shift"]');
      expect(line, 'floor my-shift line exists').not.toBeNull();
      expect(line?.textContent).toContain('Shift C · 4/11');
    });

    it('missing businessDate shows only shiftName', async () => {
      setLang('th');
      const shift = makeStaffFloorMyShift({
        shiftName: 'C',
        businessDate: undefined,
      });
      dispose = render(() => <FloorCard ctx={ctx({ myShift: shift })} />, host);
      await flush();

      const info = host.querySelector('[data-testid="floor-shift-info"]');
      expect(info?.textContent?.trim()).toBe('กะ C');
    });

    it('missing shiftName shows only formatted businessDate', async () => {
      const shift = makeStaffFloorMyShift({
        shiftName: undefined,
        businessDate: '2026-11-04',
      });
      dispose = render(() => <FloorCard ctx={ctx({ myShift: shift })} />, host);
      await flush();

      const info = host.querySelector('[data-testid="floor-shift-info"]');
      expect(info?.textContent?.trim()).toBe('4/11');
    });
  });

  describe('F4: onBreak badge', () => {
    it('th: onBreak true shows "พักเบรก" badge', async () => {
      setLang('th');
      const shift = makeStaffFloorMyShift({
        onBreak: true,
      });
      dispose = render(() => <FloorCard ctx={ctx({ myShift: shift })} />, host);
      await flush();

      const badge = host.querySelector('[data-testid="floor-break-badge"]');
      expect(badge, 'break badge exists').not.toBeNull();
      expect(badge?.textContent?.trim()).toBe('พักเบรก');
    });

    it('en: onBreak true shows "On break" badge', async () => {
      setLang('en');
      const shift = makeStaffFloorMyShift({
        onBreak: true,
      });
      dispose = render(() => <FloorCard ctx={ctx({ myShift: shift })} />, host);
      await flush();

      const badge = host.querySelector('[data-testid="floor-break-badge"]');
      expect(badge, 'break badge exists').not.toBeNull();
      expect(badge?.textContent?.trim()).toBe('On break');
    });

    it('onBreak false does not show badge', async () => {
      const shift = makeStaffFloorMyShift({
        shiftName: 'C',
        businessDate: '2026-11-04',
        onBreak: false,
      });
      dispose = render(() => <FloorCard ctx={ctx({ myShift: shift })} />, host);
      await flush();

      expect(host.querySelector('[data-testid="floor-break-badge"]')).toBeNull();
      expect(host.querySelector('[data-testid="floor-shift-info"]')?.textContent?.trim()).toBe('กะ C · 4/11');
    });

    it('shift info and onBreak badge appear together', async () => {
      setLang('th');
      const shift = makeStaffFloorMyShift({
        shiftName: 'C',
        businessDate: '2026-11-04',
        onBreak: true,
      });
      dispose = render(() => <FloorCard ctx={ctx({ myShift: shift })} />, host);
      await flush();

      const line = host.querySelector('[data-testid="floor-my-shift"]');
      expect(line?.textContent).toContain('กะ C · 4/11');
      expect(line?.textContent).toContain('พักเบรก');
      expect(host.querySelector('[data-testid="floor-break-badge"]')).not.toBeNull();
    });
  });

  describe('F5: none of the three fields — the line is as today', () => {
    it('when myShift has none of shiftName, businessDate, onBreak, nothing extra is shown', async () => {
      const shift = makeStaffFloorMyShift({
        id: 'SH-1',
        openedAt: '2026-11-04T01:05:00Z',
        shiftName: undefined,
        businessDate: undefined,
        onBreak: undefined,
      });
      dispose = render(() => <FloorCard ctx={ctx({ myShift: shift })} />, host);
      await flush();

      expect(host.querySelector('[data-testid="floor-my-shift"]')).toBeNull();
      expect(host.querySelector('[data-testid="floor-break-badge"]')).toBeNull();
      expect(host.querySelector('[data-testid="floor-shift-info"]')).toBeNull();
    });

    it('when myShift is null, nothing is shown', async () => {
      dispose = render(() => <FloorCard ctx={ctx({ myShift: null })} />, host);
      await flush();

      expect(host.querySelector('[data-testid="floor-my-shift"]')).toBeNull();
    });
  });
});
