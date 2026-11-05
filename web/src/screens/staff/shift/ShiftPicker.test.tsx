import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import type { StaffCtx } from '../cards';
import {
  makeOpenShiftChoice,
  makeOpenShiftChoices,
  makeStaffFloorView,
  makeStaffMeView,
} from '../../../shared/testing/fixtures';
import { ShiftPicker, formatChoiceLabel, shiftChosenOrUnneeded } from './ShiftPicker';
import type { OpenShiftChoices } from '../../../shared/testing/fixtures/open_shift_choices';

const BRANCH = 'MAIN';
const ME = '00000000-0000-4000-8000-0000000000a1';

function mockCtx(): StaffCtx {
  return {
    me: makeStaffMeView({
      userId: ME,
      name: 'Somchai',
      roles: [],
      branchId: BRANCH,
      branchName: 'XC-B',
      permissions: ['ShiftOperate'],
    }),
    floor: makeStaffFloorView({
      phase: 'OPEN',
      hours: null,
      myClockIn: '2026-11-04T00:00:00Z',
      myShift: null,
      cutoff: null,
      target: null,
      safe: null,
    }),
    refresh: vi.fn(async () => {}),
  };
}

function jsonResponse(status: number, data: unknown): Response {
  return new Response(JSON.stringify(data), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

describe('ShiftPicker (CS-15 #6216)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
    setLang('th');
  });

  afterEach(() => {
    if (dispose) {
      dispose();
      dispose = undefined;
    }
    host.remove();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  function stubView(payload: OpenShiftChoices) {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/open_shift_choices')) {
          return jsonResponse(200, { version: 1, changed: true, data: payload });
        }
        return jsonResponse(404, { error: { code: 'notFound', message: 'not found' } });
      }),
    );
  }

  // U2: one choice
  it('U2 one choice: preselected, onChange called with it, label formatted as "A · 06:00–14:00"', async () => {
    const choiceA = makeOpenShiftChoice({
      templateId: 'shift-a',
      name: 'A',
      startsAt: '2026-11-03T23:00:00Z', // 06:00 Bangkok
      endsAt: '2026-11-04T07:00:00Z',   // 14:00 Bangkok
    });
    stubView(makeOpenShiftChoices({ choices: [choiceA], preselect: 'shift-a' }));

    const onChange = vi.fn();
    let selectedId: () => string | null = () => null;

    dispose = render(() => {
      const [val, setVal] = createSignal<string | null>(null);
      selectedId = val;
      return (
        <ShiftPicker
          ctx={mockCtx()}
          value={val}
          onChange={(id) => {
            setVal(id);
            onChange(id);
          }}
        />
      );
    }, host);

    await vi.waitFor(() => {
      expect(onChange).toHaveBeenCalledWith('shift-a');
    });

    const radio = host.querySelector('input[type="radio"][value="shift-a"]') as HTMLInputElement;
    expect(radio).not.toBeNull();
    expect(radio.checked).toBe(true);
    expect(host.textContent).toContain('A · 06:00–14:00');
    expect(selectedId()).toBe('shift-a');
    expect(shiftChosenOrUnneeded([choiceA], selectedId())).toBe(true);
  });

  // U3: two choices
  it('U3 two choices: none selected; shiftChosenOrUnneeded false until one picked', async () => {
    const choiceA = makeOpenShiftChoice({
      templateId: 'shift-a',
      name: 'A',
      startsAt: '2026-11-03T23:00:00Z', // 06:00 Bangkok
      endsAt: '2026-11-04T07:00:00Z',   // 14:00 Bangkok
    });
    const choiceB = makeOpenShiftChoice({
      templateId: 'shift-b',
      name: 'B',
      startsAt: '2026-11-04T07:00:00Z', // 14:00 Bangkok
      endsAt: '2026-11-04T15:00:00Z',   // 22:00 Bangkok
    });
    stubView(makeOpenShiftChoices({ choices: [choiceA, choiceB], preselect: null }));

    const onChange = vi.fn();
    let selectedId: () => string | null = () => null;

    dispose = render(() => {
      const [val, setVal] = createSignal<string | null>(null);
      selectedId = val;
      return (
        <ShiftPicker
          ctx={mockCtx()}
          value={val}
          onChange={(id) => {
            setVal(id);
            onChange(id);
          }}
        />
      );
    }, host);

    await vi.waitFor(() => {
      const radios = host.querySelectorAll('input[type="radio"]');
      expect(radios.length).toBe(2);
    });

    // None is selected initially, onChange not called
    expect(onChange).not.toHaveBeenCalled();
    const radioA = host.querySelector('input[type="radio"][value="shift-a"]') as HTMLInputElement;
    const radioB = host.querySelector('input[type="radio"][value="shift-b"]') as HTMLInputElement;
    expect(radioA.checked).toBe(false);
    expect(radioB.checked).toBe(false);
    expect(shiftChosenOrUnneeded([choiceA, choiceB], selectedId())).toBe(false);

    expect(host.textContent).toContain('A · 06:00–14:00');
    expect(host.textContent).toContain('B · 14:00–22:00');

    // Pick choice B
    radioB.click();

    await vi.waitFor(() => {
      expect(onChange).toHaveBeenCalledWith('shift-b');
      expect(selectedId()).toBe('shift-b');
      expect(radioB.checked).toBe(true);
    });
    expect(shiftChosenOrUnneeded([choiceA, choiceB], selectedId())).toBe(true);
  });

  // U4: none
  it('U4 none: the outside-hours text; onChange(null)', async () => {
    stubView(makeOpenShiftChoices({ choices: [], preselect: null }));

    const onChange = vi.fn();

    dispose = render(() => {
      const [val, setVal] = createSignal<string | null>('previous-value');
      return (
        <ShiftPicker
          ctx={mockCtx()}
          value={val}
          onChange={(id) => {
            setVal(id);
            onChange(id);
          }}
        />
      );
    }, host);

    await vi.waitFor(() => {
      expect(onChange).toHaveBeenCalledWith(null);
    });

    // In Thai by default
    expect(host.textContent).toContain('นอกเวลากะ');
    expect(host.querySelectorAll('input[type="radio"]').length).toBe(0);
    expect(shiftChosenOrUnneeded([], null)).toBe(false);

    // Also verify in English
    setLang('en');
    expect(host.textContent).toContain('Outside shift hours');
  });

  describe('shiftChosenOrUnneeded helper', () => {
    const choiceA = makeOpenShiftChoice({ templateId: 't-1', name: 'A' });
    const choiceB = makeOpenShiftChoice({ templateId: 't-2', name: 'B' });

    it('returns true when exactly one choice exists regardless of value', () => {
      expect(shiftChosenOrUnneeded([choiceA], null)).toBe(true);
      expect(shiftChosenOrUnneeded([choiceA], undefined)).toBe(true);
      expect(shiftChosenOrUnneeded([choiceA], 't-1')).toBe(true);
      expect(shiftChosenOrUnneeded(makeOpenShiftChoices({ choices: [choiceA] }), null)).toBe(true);
    });

    it('returns true when multiple choices exist and value matches one', () => {
      expect(shiftChosenOrUnneeded([choiceA, choiceB], 't-1')).toBe(true);
      expect(shiftChosenOrUnneeded([choiceA, choiceB], 't-2')).toBe(true);
      expect(shiftChosenOrUnneeded([choiceA, choiceB], () => 't-2')).toBe(true);
    });

    it('returns false when multiple choices exist and value is null or does not match', () => {
      expect(shiftChosenOrUnneeded([choiceA, choiceB], null)).toBe(false);
      expect(shiftChosenOrUnneeded([choiceA, choiceB], undefined)).toBe(false);
      expect(shiftChosenOrUnneeded([choiceA, choiceB], 't-3')).toBe(false);
      expect(shiftChosenOrUnneeded([choiceA, choiceB], () => null)).toBe(false);
    });

    it('returns false when no choices exist or choices is null/undefined', () => {
      expect(shiftChosenOrUnneeded([], null)).toBe(false);
      expect(shiftChosenOrUnneeded([], 't-1')).toBe(false);
      expect(shiftChosenOrUnneeded(null, 't-1')).toBe(false);
      expect(shiftChosenOrUnneeded(undefined, 't-1')).toBe(false);
    });
  });

  describe('formatChoiceLabel helper', () => {
    it('formats ISO timestamps to Asia/Bangkok time with en-dash', () => {
      const choice = makeOpenShiftChoice({
        name: 'Morning',
        startsAt: '2026-11-03T23:00:00Z',
        endsAt: '2026-11-04T07:00:00Z',
      });
      expect(formatChoiceLabel(choice)).toBe('Morning · 06:00–14:00');
    });

    it('accepts direct time strings', () => {
      const choice = makeOpenShiftChoice({
        name: 'C',
        startsAt: '22:00',
        endsAt: '06:00',
      });
      expect(formatChoiceLabel(choice)).toBe('C · 22:00–06:00');
    });
  });
});
