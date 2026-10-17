// #2427: Admin Booking Slots — a second click while a write is in flight must not resend
// (behaviour table rows 1-4 and edge cases of the ticket).
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { AdminBookingSlotsData } from '../../../shared/api/generated/admin_booking_slots';
import type { Me } from '../../../shared/api/generated/common';
import { setLang } from '../../../shared/i18n';
import { sharedSession } from '../../../shared/session/session';
import { meOf } from '../../../test-support/me';
import type { AdminMenuProps } from '../menus';
import { BookingSlots } from './BookingSlots';
import * as table from './i18n';

const B01 = 'XC-B01';
const B02 = 'XC-B02';
const VIEW = '/api/v3/views/admin_booking_slots';
const SAVE_RULE = `/api/branches/${B01}/booking-slot-rule`;
const ADD_CLOSURE = `/api/branches/${B01}/closures`;
const CLOSE_SLOT = '/api/booking-slots/slot-a-1/close';
const SAVE_POLICY = '/api/booking-policy';

const BRANCH_A_DATA: AdminBookingSlotsData = {
  rule: { branchId: B01, slotMinutes: 30, capacity: 8, horizonDays: 21 },
  closures: [{ branchId: B01, closedOn: '2026-12-25', reason: 'Christmas Day' }],
  slots: [
    {
      id: 'slot-a-1',
      branchId: B01,
      serviceDate: '2026-10-15',
      startAt: '2026-10-15T09:00:00Z',
      endAt: '2026-10-15T09:30:00Z',
      capacity: 8,
      reserved: 2,
      status: 'OPEN',
    },
  ],
  policy: { cancelCutoffMinutes: 45, noShowGraceMinutes: 20, confirmExpiryMinutes: 30 },
};

const BRANCH_B_DATA: AdminBookingSlotsData = {
  rule: { branchId: B02, slotMinutes: 60, capacity: 12, horizonDays: 14 },
  closures: [{ branchId: B02, closedOn: '2026-04-13', reason: 'Songkran Festival' }],
  slots: [],
  policy: BRANCH_A_DATA.policy,
};

type Reply = { status: number; body: unknown } | 'network';

interface SentWrite {
  method: string;
  path: string;
  key: string | null;
  body: string;
}

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

const WRITE_BUTTONS = [
  'saveRule',
  'addClosure',
  'removeClosure',
  'updateCapacity',
  'close',
  'open',
  'savePolicy',
] as const;
type WriteButton = (typeof WRITE_BUTTONS)[number];

describe('#2427: BookingSlots blocks a second write while one is in flight', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let writes: SentWrite[];
  let hold: boolean;
  let held: Deferred<Reply>[];
  let pendingKeys: Set<string>;

  beforeEach(() => {
    setLang('th');
    sharedSession().updateMe(meOf({ id: 'admin-1', role: 'ADMIN', branch: B01, name: 'Admin', orgWide: true }));
    container = document.createElement('div');
    document.body.appendChild(container);
    writes = [];
    hold = false;
    held = [];
    pendingKeys = new Set();

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
      const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(raw, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();

      if (method === 'GET' && u.pathname === VIEW) {
        const data = u.searchParams.get('branch') === B02 ? BRANCH_B_DATA : BRANCH_A_DATA;
        return json(200, { version: 1, changed: true, data: structuredClone(data) });
      }

      const key = new Headers(init?.headers).get('idempotency-key');
      const body = typeof init?.body === 'string' ? init.body : '';
      writes.push({ method, path: u.pathname, key, body });

      // Server fact (pg_store.rs): a key that is still PENDING answers 409 idempotencyConflict.
      if (key && pendingKeys.has(key)) {
        return json(409, { error: { code: 'idempotencyConflict', message: 'request in progress' } });
      }
      if (key) pendingKeys.add(key);
      try {
        let reply: Reply = { status: 200, body: {} };
        if (hold) {
          const d = deferred<Reply>();
          held.push(d);
          reply = await d.promise;
        }
        if (reply === 'network') throw new TypeError('Failed to fetch');
        return json(reply.status, reply.body);
      } finally {
        if (key) pendingKeys.delete(key);
      }
    });
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    container.remove();
    sharedSession().updateMe(undefined as unknown as Me);
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

  async function mount(): Promise<{ setBranch: (b: string) => void }> {
    const [branch, setBranch] = createSignal(B01);
    const [period] = createSignal({ from: '2026-10-15', to: '2026-10-15' });
    const [shown] = createSignal(true);
    const props: AdminMenuProps = { branch, period, shown };
    dispose = render(() => <BookingSlots {...props} />, container);
    await settle();
    return { setBranch };
  }

  function button(name: WriteButton): HTMLButtonElement {
    const btn = Array.from(container.querySelectorAll('button')).find(
      (b) => b.textContent?.trim() === table.th[name],
    );
    expect(btn, `${name} button exists`).toBeDefined();
    return btn!;
  }

  function input(name: string): HTMLInputElement {
    const el = container.querySelector<HTMLInputElement>(`input[name="${name}"]`);
    expect(el, `${name} input exists`).not.toBeNull();
    return el!;
  }

  function setInputValue(el: HTMLInputElement, value: string): void {
    el.value = value;
    el.dispatchEvent(new Event('input', { bubbles: true }));
  }

  function alertEl(): Element | null {
    return container.querySelector('[role="alert"]');
  }

  function writesTo(path: string): SentWrite[] {
    return writes.filter((w) => w.path === path);
  }

  async function release(reply: Reply): Promise<void> {
    const d = held.shift();
    expect(d, 'a write is held').toBeDefined();
    d!.resolve(reply);
    await settle();
  }

  async function fillClosure(): Promise<void> {
    setInputValue(input('closedOn'), '2026-12-31');
    setInputValue(input('reason'), 'New Year Eve');
  }

  it('row 1: double click Add closure -> one closure-add POST, no error', async () => {
    await mount();
    await fillClosure();
    hold = true;

    button('addClosure').click();
    await settle();
    button('addClosure').click();
    await settle();
    await release({ status: 200, body: {} });

    expect(writesTo(ADD_CLOSURE)).toHaveLength(1);
    expect(alertEl()).toBeNull();
  });

  it("row 2: click a slot's Close while that close is in flight -> one close POST, no error", async () => {
    await mount();
    hold = true;

    button('close').click();
    await settle();
    button('close').click();
    await settle();
    await release({ status: 200, body: {} });

    expect(writesTo(CLOSE_SLOT)).toHaveLength(1);
    expect(alertEl()).toBeNull();
  });

  it('row 3: network error on Save policy, then click again -> same key, byte-equal body', async () => {
    await mount();
    hold = true;

    button('savePolicy').click();
    await settle();
    // A click while the first save is in flight must not send anything.
    button('savePolicy').click();
    await settle();
    await release('network');

    hold = false;
    button('savePolicy').click();
    await settle();

    const puts = writesTo(SAVE_POLICY);
    expect(puts).toHaveLength(2);
    expect(puts[0]!.key).toBeTruthy();
    expect(puts[1]!.key).toBe(puts[0]!.key);
    expect(puts[1]!.body).toBe(puts[0]!.body);
  });

  it('row 4: after a successful write every write button is enabled again (and disabled while in flight)', async () => {
    await mount();
    await fillClosure();

    for (const action of WRITE_BUTTONS) {
      hold = true;
      if (action === 'addClosure') await fillClosure();
      button(action).click();
      await settle();

      for (const name of WRITE_BUTTONS) {
        expect(button(name).disabled, `${name} disabled while ${action} is in flight`).toBe(true);
      }

      hold = false;
      await release({ status: 200, body: {} });

      for (const name of WRITE_BUTTONS) {
        expect(button(name).disabled, `${name} enabled after ${action} answered`).toBe(false);
      }
    }
    expect(held).toHaveLength(0);
  });

  it('edge: writing is cleared after a refusal -> the next click sends again', async () => {
    await mount();
    await fillClosure();
    hold = true;

    button('addClosure').click();
    await settle();
    expect(button('addClosure').disabled).toBe(true);

    hold = false;
    await release({ status: 400, body: { error: { code: 'validationFailed', message: 'refused' } } });

    expect(alertEl()).not.toBeNull();
    expect(button('addClosure').disabled).toBe(false);

    button('addClosure').click();
    await settle();

    expect(writesTo(ADD_CLOSURE)).toHaveLength(2);
  });

  it('edge: a branch switch while a write is in flight still follows the branch-switch read rules', async () => {
    const { setBranch } = await mount();
    hold = true;

    button('saveRule').click();
    await settle();
    expect(button('saveRule').disabled).toBe(true);

    setBranch(B02);
    await settle();

    expect(input('slotMinutes').value).toBe('60');
    expect(input('capacity').value).toBe('12');
    expect(input('horizonDays').value).toBe('14');
    expect(container.textContent).toContain('Songkran Festival');
    expect(container.textContent).not.toContain('Christmas Day');
    // The old write is still in flight, so writes stay blocked on the new branch too.
    expect(button('saveRule').disabled).toBe(true);
    button('saveRule').click();
    await settle();
    expect(writesTo(SAVE_RULE)).toHaveLength(1);

    hold = false;
    await release({ status: 200, body: {} });

    expect(input('slotMinutes').value).toBe('60');
    expect(container.textContent).toContain('Songkran Festival');
    expect(container.textContent).not.toContain('Christmas Day');
    expect(button('saveRule').disabled).toBe(false);
    expect(alertEl()).toBeNull();
  });
});
