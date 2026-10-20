import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang } from '../../../shared/i18n';
import { sharedSession } from '../../../shared/session/session';
import {
  makeAdminBookingPolicyDto,
  makeAdminBookingSlotsData,
  makeAdminClosureDto,
  makeAdminSlotDto,
  makeAdminSlotRuleDto,
} from '../../../shared/testing/fixtures';
import { meOf } from '../../../test-support/me';
import type { AdminMenuProps } from '../menus';
import { BookingSlots } from './BookingSlots';
import * as table from './i18n';

const B01 = 'XC-B01';
const B02 = 'XC-B02';
const VIEW = '/api/v3/views/admin_booking_slots';

const BRANCH_A_DATA = makeAdminBookingSlotsData({
  rule: makeAdminSlotRuleDto({ branchId: B01, slotMinutes: 30, capacity: 8, horizonDays: 21 }),
  closures: [makeAdminClosureDto({ branchId: B01, closedOn: '2026-12-25', reason: 'Christmas Day' })],
  slots: [
    makeAdminSlotDto({
      id: 'slot-a-1',
      branchId: B01,
      serviceDate: '2026-10-15',
      startAt: '2026-10-15T09:00:00Z',
      endAt: '2026-10-15T09:30:00Z',
      capacity: 8,
      reserved: 2,
      status: 'OPEN',
    }),
    makeAdminSlotDto({
      id: 'slot-a-2',
      branchId: B01,
      serviceDate: '2026-10-15',
      startAt: '2026-10-15T09:30:00Z',
      endAt: '2026-10-15T10:00:00Z',
      capacity: 8,
      reserved: 0,
      status: 'CLOSED',
    }),
  ],
  policy: makeAdminBookingPolicyDto({ cancelCutoffMinutes: 45, noShowGraceMinutes: 20, confirmExpiryMinutes: 30 }),
});

const BRANCH_B_DATA = makeAdminBookingSlotsData({
  rule: makeAdminSlotRuleDto({ branchId: B02, slotMinutes: 60, capacity: 12, horizonDays: 14 }),
  closures: [makeAdminClosureDto({ branchId: B02, closedOn: '2026-04-13', reason: 'Songkran Festival' })],
  slots: [],
  policy: BRANCH_A_DATA.policy,
});

type Reply = { status: number; body: unknown } | 'network';

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

describe('#2663: BookingSlots write answers across branch switch', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let hold: boolean;
  let held: Deferred<Reply>[];

  beforeEach(() => {
    setLang('th');
    sharedSession().updateMe(meOf({ id: 'admin-1', role: 'ADMIN', branch: B01, name: 'Admin', orgWide: true }));
    container = document.createElement('div');
    document.body.appendChild(container);
    hold = false;
    held = [];

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
      const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(raw, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();

      if (method === 'GET' && u.pathname === VIEW) {
        const data = u.searchParams.get('branch') === B02 ? BRANCH_B_DATA : BRANCH_A_DATA;
        return json(200, { version: 1, changed: true, data: structuredClone(data) });
      }

      if (hold) {
        const d = deferred<Reply>();
        held.push(d);
        const reply = await d.promise;
        if (reply === 'network') throw new TypeError('Failed to fetch');
        return json(reply.status, reply.body);
      }

      return json(200, {});
    });
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    container.remove();
    sharedSession().updateMe(undefined as never);
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  function json(status: number, body: unknown): Response {
    return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
  }

  async function settle(): Promise<void> {
    for (let i = 0; i < 10; i++) {
      await new Promise((r) => setTimeout(r, 0));
    }
  }

  function unmount(): void {
    if (dispose) {
      dispose();
      dispose = undefined;
    }
    container.innerHTML = '';
  }

  async function mount(): Promise<{ setBranch: (b: string) => void }> {
    unmount();
    const [branch, setBranch] = createSignal(B01);
    const [period] = createSignal({ from: '2026-10-15', to: '2026-10-15' });
    const [shown] = createSignal(true);
    const props: AdminMenuProps = { branch, period, shown };
    dispose = render(() => <BookingSlots {...props} />, container);
    await settle();
    return { setBranch };
  }

  function button(label: string): HTMLButtonElement {
    const btn = Array.from(container.querySelectorAll('button')).find(
      (b) => b.textContent?.trim() === label,
    );
    expect(btn, `button with label "${label}" exists`).toBeDefined();
    return btn!;
  }

  function input(name: string): HTMLInputElement {
    const el = container.querySelector<HTMLInputElement>(`input[name="${name}"]`);
    expect(el, `input "${name}" exists`).not.toBeNull();
    return el!;
  }

  function setInputValue(el: HTMLInputElement, value: string): void {
    el.value = value;
    el.dispatchEvent(new Event('input', { bubbles: true }));
  }

  function alertEl(): Element | null {
    return container.querySelector('[role="alert"]');
  }

  async function release(reply: Reply): Promise<void> {
    const d = held.shift();
    expect(d, 'a write is held').toBeDefined();
    d!.resolve(reply);
    await settle();
  }

  it('row 1: Save rule on A, switch to B, A\'s write fails (4xx and 500) -> No error on B; B\'s rule fields stay B\'s', async () => {
    // Test 4xx refusal
    const { setBranch } = await mount();
    hold = true;

    button(table.th.saveRule).click();
    await settle();

    // Switch to branch B
    setBranch(B02);
    await settle();

    expect(input('slotMinutes').value).toBe(String(BRANCH_B_DATA.rule?.slotMinutes));
    expect(input('capacity').value).toBe(String(BRANCH_B_DATA.rule?.capacity));
    expect(input('horizonDays').value).toBe(String(BRANCH_B_DATA.rule?.horizonDays));

    // Release with 400
    await release({ status: 400, body: { error: { code: 'validationFailed', message: 'Rule invalid on branch A' } } });

    expect(alertEl()).toBeNull();
    expect(input('slotMinutes').value).toBe(String(BRANCH_B_DATA.rule?.slotMinutes));
    expect(input('capacity').value).toBe(String(BRANCH_B_DATA.rule?.capacity));
    expect(input('horizonDays').value).toBe(String(BRANCH_B_DATA.rule?.horizonDays));

    // Test 500 server error
    setBranch(B01);
    await settle();
    button(table.th.saveRule).click();
    await settle();

    setBranch(B02);
    await settle();
    await release({ status: 500, body: { error: { code: 'internal', message: 'Server error on branch A' } } });

    expect(alertEl()).toBeNull();
    expect(input('slotMinutes').value).toBe(String(BRANCH_B_DATA.rule?.slotMinutes));
    expect(input('capacity').value).toBe(String(BRANCH_B_DATA.rule?.capacity));
    expect(input('horizonDays').value).toBe(String(BRANCH_B_DATA.rule?.horizonDays));
  });

  it('row 2: Add closure on A, switch to B, type a closure for B, A\'s write succeeds -> B\'s typed date and reason are kept; no error', async () => {
    const { setBranch } = await mount();
    setInputValue(input('closedOn'), '2026-12-31');
    setInputValue(input('reason'), 'New Year Eve on A');
    hold = true;

    button(table.th.addClosure).click();
    await settle();

    // Switch to branch B
    setBranch(B02);
    await settle();

    // Type a closure for B
    setInputValue(input('closedOn'), '2026-05-01');
    setInputValue(input('reason'), 'Labour Day on B');

    // A's write succeeds
    await release({ status: 200, body: {} });

    // B's typed values are preserved and no error
    expect(input('closedOn').value).toBe('2026-05-01');
    expect(input('reason').value).toBe('Labour Day on B');
    expect(alertEl()).toBeNull();
  });

  it('Add closure on A, switch to B, A\'s write fails (4xx) -> no error on B', async () => {
    const { setBranch } = await mount();
    setInputValue(input('closedOn'), '2026-12-31');
    setInputValue(input('reason'), 'New Year Eve on A');
    hold = true;

    button(table.th.addClosure).click();
    await settle();

    // Switch to branch B
    setBranch(B02);
    await settle();

    // A's write fails
    await release({
      status: 400,
      body: { error: { code: 'validationFailed', message: 'Closure invalid on branch A' } },
    });

    // No error shown on B
    expect(alertEl()).toBeNull();
  });

  it('edge: Switch A -> B -> A before the answer lands -> A\'s error is shown', async () => {
    const { setBranch } = await mount();
    setInputValue(input('closedOn'), '2026-12-31');
    setInputValue(input('reason'), 'New Year Eve on A');
    hold = true;

    button(table.th.addClosure).click();
    await settle();

    // Switch A -> B -> A
    setBranch(B02);
    await settle();
    setBranch(B01);
    await settle();

    // A's write fails
    await release({
      status: 400,
      body: { error: { code: 'validationFailed', message: 'Closure invalid on branch A' } },
    });

    // Error is shown on A because branch is A again
    expect(alertEl()).not.toBeNull();
  });

  it('row 3: Remove closure / Update capacity / Close slot / Open slot on A, switch to B, write fails -> No error on B', async () => {
    const actions: Array<{ name: string; getButton: () => HTMLButtonElement }> = [
      { name: 'removeClosure', getButton: () => button(table.th.removeClosure) },
      { name: 'updateCapacity', getButton: () => button(table.th.updateCapacity) },
      { name: 'closeSlot', getButton: () => button(table.th.close) },
      { name: 'openSlot', getButton: () => button(table.th.open) },
    ];

    for (const action of actions) {
      const { setBranch } = await mount();
      hold = true;

      action.getButton().click();
      await settle();

      // Switch to branch B
      setBranch(B02);
      await settle();

      // A's write fails
      await release({ status: 400, body: { error: { code: 'validationFailed', message: `${action.name} failed on A` } } });

      expect(alertEl(), `alert should be null after ${action.name} fails on branch A while on branch B`).toBeNull();

      dispose?.();
      dispose = undefined;
      container.innerHTML = '';
    }
  });

  it('row 4: The same writes fail with no switch -> The worded error is shown as today', async () => {
    // Save rule
    const { setBranch: _ } = await mount();
    hold = true;
    button(table.th.saveRule).click();
    await settle();
    await release({ status: 400, body: { error: { code: 'validationFailed', message: 'Rule invalid' } } });
    expect(alertEl()).not.toBeNull();

    // Add closure
    await mount();
    setInputValue(input('closedOn'), '2026-12-31');
    setInputValue(input('reason'), 'New Year Eve');
    button(table.th.addClosure).click();
    await settle();
    await release({ status: 400, body: { error: { code: 'validationFailed', message: 'Closure invalid' } } });
    expect(alertEl()).not.toBeNull();

    // Remove closure
    await mount();
    button(table.th.removeClosure).click();
    await settle();
    await release({ status: 400, body: { error: { code: 'validationFailed', message: 'Remove failed' } } });
    expect(alertEl()).not.toBeNull();

    // Update capacity
    await mount();
    button(table.th.updateCapacity).click();
    await settle();
    await release({ status: 400, body: { error: { code: 'validationFailed', message: 'Update capacity failed' } } });
    expect(alertEl()).not.toBeNull();

    // Close slot
    await mount();
    button(table.th.close).click();
    await settle();
    await release({ status: 400, body: { error: { code: 'validationFailed', message: 'Close slot failed' } } });
    expect(alertEl()).not.toBeNull();

    // Open slot
    await mount();
    button(table.th.open).click();
    await settle();
    await release({ status: 400, body: { error: { code: 'validationFailed', message: 'Open slot failed' } } });
    expect(alertEl()).not.toBeNull();
  });

  it('row 5: Save policy fails after a switch -> The error is still shown', async () => {
    const { setBranch } = await mount();
    hold = true;

    button(table.th.savePolicy).click();
    await settle();

    // Switch branch to B02
    setBranch(B02);
    await settle();

    // Policy save fails
    await release({ status: 400, body: { error: { code: 'validationFailed', message: 'Policy invalid' } } });

    // Since policy is org-wide, error is still shown on B
    expect(alertEl()).not.toBeNull();
  });
});
