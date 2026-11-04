// @vitest-environment jsdom
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiClient } from '../../../shared/api/client';
import type { BranchShiftsData } from '../../../shared/api/generated/branch_shifts';
import { setLang } from '../../../shared/i18n';
import { BranchesScreen } from './BranchesScreen';
import { ShiftsEditor } from './ShiftsEditor';

const DEFAULT_SHIFTS_DATA: BranchShiftsData = {
  templates: [
    {
      id: 'shift-a',
      name: 'A',
      startsAt: '06:00',
      endsAt: '14:00',
      breakStartsAt: null,
      breakEndsAt: null,
    },
    {
      id: 'shift-b',
      name: 'B',
      startsAt: '14:00',
      endsAt: '22:00',
      breakStartsAt: null,
      breakEndsAt: null,
    },
    {
      id: 'shift-c',
      name: 'C',
      startsAt: '22:00',
      endsAt: '06:00',
      breakStartsAt: null,
      breakEndsAt: null,
    },
  ],
  cutover: '06:00',
  defaultShiftHours: 8,
};

function flush(): Promise<void> {
  return new Promise((r) => setTimeout(r, 0));
}

function setInputValue(input: HTMLInputElement, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
}

describe('ShiftsEditor (CS-14b #6215)', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setLang('th');
    container = document.createElement('div');
    document.body.appendChild(container);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    container.remove();
    vi.unstubAllGlobals();
  });

  it('U1: loads the defaults → three rows A/B/C, no red gap, Save enabled, cutover 06:00', async () => {
    const fakeFetch: typeof fetch = async (input) => {
      const url = String(input);
      if (url.includes('/api/v3/views/branch_shifts')) {
        return new Response(
          JSON.stringify({ version: 'v1', changed: true, data: DEFAULT_SHIFTS_DATA }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }
      return new Response('{}', { status: 404 });
    };

    const api = new ApiClient(fakeFetch);
    dispose = render(() => <ShiftsEditor branchId="b1" api={api} />, container);
    await flush();
    await flush();

    const rows = container.querySelectorAll<HTMLElement>('[data-shift-row]');
    expect(rows.length).toBe(3);

    const names = Array.from(rows).map(
      (r, i) => r.querySelector<HTMLInputElement>(`input[name="shift-name-${i}"]`)?.value,
    );
    expect(names).toEqual(['A', 'B', 'C']);

    const gaps = container.querySelectorAll('.branches-shifts-bar-gap');
    expect(gaps.length).toBe(0);

    const cutoverEl = container.querySelector<HTMLElement>('[data-shifts-cutover]');
    expect(cutoverEl?.textContent).toBe('วันทำการเริ่ม 06:00');

    const saveBtn = container.querySelector<HTMLButtonElement>('[data-action="shifts-save"]')!;
    expect(saveBtn.disabled).toBe(false);
  });

  it('U1b: add a row starting 09:00 → its end shows 17:00 (from defaultShiftHours)', async () => {
    const fakeFetch: typeof fetch = async () =>
      new Response(
        JSON.stringify({ version: 'v1', changed: true, data: DEFAULT_SHIFTS_DATA }),
        { status: 200, headers: { 'content-type': 'application/json' } },
      );

    const api = new ApiClient(fakeFetch);
    dispose = render(() => <ShiftsEditor branchId="b1" api={api} />, container);
    await flush();
    await flush();

    const addBtn = container.querySelector<HTMLButtonElement>('[data-action="shifts-add"]')!;
    addBtn.click();

    const start3 = container.querySelector<HTMLInputElement>('input[name="shift-start-3"]')!;
    const end3 = container.querySelector<HTMLInputElement>('input[name="shift-end-3"]')!;
    expect(start3.value).toBe('09:00');
    expect(end3.value).toBe('17:00');

    // Changing start before touching end updates end via defaultEnd(start, defaultShiftHours)
    setInputValue(start3, '10:00');
    expect(end3.value).toBe('18:00');
  });

  it('U1b (non-hardcoded defaultShiftHours): uses defaultShiftHours from view when not 8', async () => {
    const fakeFetch: typeof fetch = async () =>
      new Response(
        JSON.stringify({
          version: 'v1',
          changed: true,
          data: { ...DEFAULT_SHIFTS_DATA, defaultShiftHours: 6 },
        }),
        { status: 200, headers: { 'content-type': 'application/json' } },
      );

    const api = new ApiClient(fakeFetch);
    dispose = render(() => <ShiftsEditor branchId="b1" api={api} />, container);
    await flush();
    await flush();

    container.querySelector<HTMLButtonElement>('[data-action="shifts-add"]')!.click();
    const start3 = container.querySelector<HTMLInputElement>('input[name="shift-start-3"]')!;
    const end3 = container.querySelector<HTMLInputElement>('input[name="shift-end-3"]')!;
    expect(start3.value).toBe('09:00');
    expect(end3.value).toBe('15:00');
  });

  it('U1c: change C to end at 04:00 → red window 04:00–06:00, Save disabled', async () => {
    const fakeFetch: typeof fetch = async () =>
      new Response(
        JSON.stringify({ version: 'v1', changed: true, data: DEFAULT_SHIFTS_DATA }),
        { status: 200, headers: { 'content-type': 'application/json' } },
      );

    const api = new ApiClient(fakeFetch);
    dispose = render(() => <ShiftsEditor branchId="b1" api={api} />, container);
    await flush();
    await flush();

    const endC = container.querySelector<HTMLInputElement>('input[name="shift-end-2"]')!;
    setInputValue(endC, '04:00');

    const gapEls = container.querySelectorAll<HTMLElement>('.branches-shifts-bar-gap');
    expect(gapEls.length).toBe(1);
    expect(gapEls[0]!.getAttribute('data-gap')).toBe('04:00-06:00');
    expect(gapEls[0]!.textContent).toContain('ช่วง 04:00–06:00 ไม่มีกะ');

    const saveBtn = container.querySelector<HTMLButtonElement>('[data-action="shifts-save"]')!;
    expect(saveBtn.disabled).toBe(true);
  });

  it('U1d: save → PUT /api/branches/shifts with all rows and their ids; the answer re-renders', async () => {
    const putBodies: unknown[] = [];

    const fakeFetch: typeof fetch = async (input, init) => {
      const url = String(input);
      if (url.includes('/api/v3/views/branch_shifts')) {
        return new Response(
          JSON.stringify({ version: 'v1', changed: true, data: DEFAULT_SHIFTS_DATA }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }
      if (url.endsWith('/api/branches/shifts') && init?.method === 'PUT') {
        putBodies.push(JSON.parse(String(init.body)));
        const updated: BranchShiftsData = {
          templates: [
            {
              id: 'shift-a',
              name: 'กะเช้า',
              startsAt: '06:00',
              endsAt: '14:00',
              breakStartsAt: '10:00',
              breakEndsAt: '10:30',
            },
            {
              id: 'shift-b',
              name: 'กะบ่าย',
              startsAt: '14:00',
              endsAt: '22:00',
              breakStartsAt: null,
              breakEndsAt: null,
            },
            {
              id: 'shift-c',
              name: 'กะดึก',
              startsAt: '22:00',
              endsAt: '06:00',
              breakStartsAt: null,
              breakEndsAt: null,
            },
          ],
          cutover: '06:00',
          defaultShiftHours: 8,
        };
        return new Response(JSON.stringify(updated), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }
      return new Response('{}', { status: 404 });
    };

    const api = new ApiClient(fakeFetch);
    dispose = render(() => <ShiftsEditor branchId="b1" api={api} />, container);
    await flush();
    await flush();

    // Edit row 0 break times
    setInputValue(
      container.querySelector<HTMLInputElement>('input[name="shift-break-start-0"]')!,
      '10:00',
    );
    setInputValue(
      container.querySelector<HTMLInputElement>('input[name="shift-break-end-0"]')!,
      '10:30',
    );

    const saveBtn = container.querySelector<HTMLButtonElement>('[data-action="shifts-save"]')!;
    saveBtn.click();
    await flush();
    await flush();

    expect(putBodies.length).toBe(1);
    expect(putBodies[0]).toEqual({
      branch: 'b1',
      templates: [
        {
          id: 'shift-a',
          name: 'A',
          startsAt: '06:00',
          endsAt: '14:00',
          breakStartsAt: '10:00',
          breakEndsAt: '10:30',
        },
        {
          id: 'shift-b',
          name: 'B',
          startsAt: '14:00',
          endsAt: '22:00',
          breakStartsAt: null,
          breakEndsAt: null,
        },
        {
          id: 'shift-c',
          name: 'C',
          startsAt: '22:00',
          endsAt: '06:00',
          breakStartsAt: null,
          breakEndsAt: null,
        },
      ],
    });

    // Server answer re-renders into the inputs
    expect(
      container.querySelector<HTMLInputElement>('input[name="shift-name-0"]')?.value,
    ).toBe('กะเช้า');
    expect(
      container.querySelector<HTMLInputElement>('input[name="shift-name-1"]')?.value,
    ).toBe('กะบ่าย');
    expect(
      container.querySelector<HTMLInputElement>('input[name="shift-name-2"]')?.value,
    ).toBe('กะดึก');
  });

  it('U1e: server answers 422 gap (and row-index error) → window text and row error show', async () => {
    let mode: 'gap' | 'row' = 'gap';

    const fakeFetch: typeof fetch = async (input, init) => {
      const url = String(input);
      if (url.includes('/api/v3/views/branch_shifts')) {
        return new Response(
          JSON.stringify({ version: 'v1', changed: true, data: DEFAULT_SHIFTS_DATA }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }
      if (url.endsWith('/api/branches/shifts') && init?.method === 'PUT') {
        if (mode === 'gap') {
          return new Response(
            JSON.stringify({
              error: {
                code: 'shiftPlanInvalid',
                message: 'shift plan is invalid',
                details: { reason: 'gap', detail: '04:00-06:00' },
              },
            }),
            { status: 422, headers: { 'content-type': 'application/json' } },
          );
        }
        return new Response(
          JSON.stringify({
            error: {
              code: 'shiftPlanInvalid',
              message: 'shift plan is invalid',
              details: { reason: 'breakOutsideShift', detail: '1' },
            },
          }),
          { status: 422, headers: { 'content-type': 'application/json' } },
        );
      }
      return new Response('{}', { status: 404 });
    };

    const api = new ApiClient(fakeFetch);
    dispose = render(() => <ShiftsEditor branchId="b1" api={api} />, container);
    await flush();
    await flush();

    const saveBtn = container.querySelector<HTMLButtonElement>('[data-action="shifts-save"]')!;
    saveBtn.click();
    await flush();
    await flush();

    const topAlert = container.querySelector<HTMLElement>('[data-shifts-error]');
    expect(topAlert).not.toBeNull();
    expect(topAlert!.textContent).toContain('ช่วง 04:00–06:00 ไม่มีกะ');

    // Now test row-index error (detail = "1")
    mode = 'row';
    saveBtn.click();
    await flush();
    await flush();

    const rowAlert = container.querySelector<HTMLElement>('[data-row-error="1"]');
    expect(rowAlert).not.toBeNull();
    expect(rowAlert!.textContent).toContain('เวลาพักต้องอยู่ภายในเวลากะ');
  });

  it('mounts inside BranchesScreen for the selected branch', async () => {
    const fakeFetch: typeof fetch = async (input) => {
      const url = String(input);
      if (url.startsWith('/api/v3/views/admin_branches_profile')) {
        return new Response(
          JSON.stringify({
            version: 1,
            changed: true,
            data: {
              branch: 'b1',
              name: 'สำนักงานใหญ่',
              code: 'HQ',
              address: '1 Main Rd',
              timezone: 'Asia/Bangkok',
              openingHours: {
                mon: [{ open: '09:00', close: '17:00' }],
                tue: [],
                wed: [],
                thu: [],
                fri: [],
                sat: [],
                sun: [],
              },
              bookingEnabled: true,
            },
          }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }
      if (url.startsWith('/api/v3/views/admin_branches')) {
        return new Response(
          JSON.stringify({
            version: 1,
            changed: true,
            data: {
              branches: [{ id: 'b1', name: 'สำนักงานใหญ่' }],
            },
          }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }
      if (url.startsWith('/api/v3/views/branch_shifts')) {
        return new Response(
          JSON.stringify({ version: 1, changed: true, data: DEFAULT_SHIFTS_DATA }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }
      return new Response('{}', { status: 404 });
    };

    vi.stubGlobal('fetch', vi.fn(fakeFetch));
    dispose = render(
      () => (
        <BranchesScreen
          branch={() => 'b1'}
          period={() => ({ from: '2026-10-01', to: '2026-10-31' })}
          shown={() => true}
        />
      ),
      container,
    );
    await flush();
    await flush();
    await flush();

    const section = container.querySelector<HTMLElement>('[data-section="branch-shifts"]');
    expect(section).not.toBeNull();
    expect(section!.querySelectorAll('[data-shift-row]').length).toBe(3);
  });
});
