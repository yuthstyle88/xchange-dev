// #4939 (T21 screen half, docs/design/web-input-validation.md): up-front tests for the branch profile and
// branch tablets inputs. One test per §3 row (BR4–BR8).
//
// Contract these tests pin:
// - The timezone control is `select[name="timezone"]`, built from `Intl.supportedValuesOf('timeZone')`; a stored
//   value missing from that list stays selected.
// - The two time inputs of an open day carry `required`; saving with a cleared time sends nothing.
// - The tablet name input (`input[name="tabletName"]`) carries `maxLength` 60.
// - A server refusal `400 {code:"validationFailed", field}` (the #4921 contract: fields `timezone` and
//   `hours.<day>`) shows through `fieldError` as a line `[data-field-error="<field>"]` at that input: inside the
//   timezone field for `timezone`, inside that day's `.branches-hours-row` for `hours.<day>`. The line names the
//   field (`input.invalid`), never the server's own text.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../shared/api/client';
import { setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import { sharedSession } from '../../../shared/session/session';
import {
  makeAdminBranchItem,
  makeAdminBranchesData,
} from '../../../shared/testing/fixtures/admin_branches';
import { makeAdminBranchProfileData } from '../../../shared/testing/fixtures/admin_branches_profile';
import { makeMe } from '../../../shared/testing/fixtures/common';
import { BranchesScreen } from './BranchesScreen';
import { BranchTablets } from './BranchTablets';
import { th } from './i18n';

const B01 = 'XC-B01';
const SERVER_TEXT = 'SERVER-TEXT validationFailed';

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

let calls: Array<{ url: string; method: string; body: unknown }>;
let storedTimezone: string;
let saveRefusalField: string | null;
const disposers: Array<() => void> = [];

async function serve(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
  const method = (init?.method ?? 'GET').toUpperCase();
  const body = init?.body ? JSON.parse(String(init.body)) : undefined;
  calls.push({ url, method, body });

  const u = new URL(url, 'http://localhost');
  if (u.pathname === '/api/v3/views/admin_branches') {
    return json({
      version: 1,
      changed: true,
      data: makeAdminBranchesData({
        branches: [makeAdminBranchItem({ id: B01, name: 'Silom Branch' })],
      }),
    });
  }
  if (u.pathname === '/api/v3/views/admin_branches_profile') {
    return json({
      version: 1,
      changed: true,
      data: makeAdminBranchProfileData({
        branch: B01,
        name: 'Silom Branch',
        code: 'SILOM',
        timezone: storedTimezone,
        openingHours: {
          mon: [{ open: '09:00', close: '17:00' }],
          tue: [{ open: '09:00', close: '17:00' }],
          wed: [],
          thu: [],
          fri: [],
          sat: [],
          sun: [],
        },
        bookingEnabled: true,
      }),
    });
  }
  if (u.pathname === '/api/branches/profile' && method === 'PUT') {
    if (saveRefusalField !== null) {
      return json(
        { error: { code: 'validationFailed', field: saveRefusalField, message: SERVER_TEXT } },
        400,
      );
    }
    return json(
      makeAdminBranchProfileData({
        branch: B01,
        name: 'Silom Branch',
        code: 'SILOM',
        timezone: (body as { timezone: string }).timezone,
      }),
    );
  }
  if (u.pathname === '/api/branch-tablets' && method === 'POST') {
    return json({ tablets: [] });
  }
  return json({ error: { code: 'notFound', message: 'not found' } }, 404);
}

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i += 1) await vi.advanceTimersByTimeAsync(0);
}

function mountScreen() {
  const [b] = createSignal(B01);
  const [shown] = createSignal(true);
  const period = () => ({ from: '2026-10-01', to: '2026-10-31' });
  const container = document.createElement('div');
  document.body.appendChild(container);
  disposers.push(
    render(() => <BranchesScreen branch={b} period={period} shown={shown} />, container),
  );
  return container;
}

function saves() {
  return calls.filter((c) => c.method === 'PUT' && c.url.includes('/api/branches/profile'));
}

function saveButton(container: HTMLElement): HTMLButtonElement {
  return container.querySelector<HTMLButtonElement>('form.branches-form button[type="submit"]')!;
}

function hoursRow(container: HTMLElement, dayLabel: string): HTMLElement {
  const rows = Array.from(container.querySelectorAll<HTMLElement>('.branches-hours-row'));
  const row = rows.find((r) => r.querySelector('.branches-hours-day')?.textContent === dayLabel);
  expect(row, `hours row for ${dayLabel}`).toBeDefined();
  return row!;
}

beforeEach(() => {
  vi.useFakeTimers({ now: new Date('2026-10-23T05:00:00Z') });
  setLang('th');
  calls = [];
  storedTimezone = 'Asia/Bangkok';
  saveRefusalField = null;
  document.body.innerHTML = '';
  sharedSession().updateMe(makeMe({ permissions: ['BranchesManage', 'SystemSettings'] }));
  vi.stubGlobal('fetch', vi.fn(serve));
});

afterEach(() => {
  while (disposers.length) disposers.pop()?.();
  document.body.innerHTML = '';
  vi.clearAllTimers();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe('#4939 branches input validation (T21 screen half)', () => {
  it('BR4: the timezone control is a select of IANA names; the stored value stays selected, even when the list lacks it', async () => {
    const container = mountScreen();
    await flush();

    const select = container.querySelector<HTMLSelectElement>('select[name="timezone"]');
    expect(select, 'timezone is a <select>').not.toBeNull();
    expect(select!.value).toBe('Asia/Bangkok');
    const options = Array.from(select!.options).map((o) => o.value);
    expect(options).toContain('Asia/Bangkok');
    expect(options).toContain('Europe/London');
    expect(options.length).toBeGreaterThanOrEqual(Intl.supportedValuesOf('timeZone').length);

    while (disposers.length) disposers.pop()?.();
    document.body.innerHTML = '';
    storedTimezone = 'Mars/Olympus_Mons';
    const second = mountScreen();
    await flush();
    const kept = second.querySelector<HTMLSelectElement>('select[name="timezone"]');
    expect(kept, 'timezone is a <select>').not.toBeNull();
    expect(kept!.value, 'a stored value missing from the list stays selected').toBe(
      'Mars/Olympus_Mons',
    );
  });

  it('BR5: an open day with a cleared time is stopped by native required; no request is sent', async () => {
    const container = mountScreen();
    await flush();

    const monOpen = hoursRow(container, th.dayMon).querySelector<HTMLInputElement>(
      'input[type="time"]',
    )!;
    expect(monOpen.required, 'the open time of an open day is required').toBe(true);
    for (const input of hoursRow(container, th.dayMon).querySelectorAll<HTMLInputElement>(
      'input[type="time"]',
    )) {
      expect(input.required, 'both time inputs of an open day are required').toBe(true);
    }

    monOpen.value = '';
    monOpen.dispatchEvent(new Event('input', { bubbles: true }));
    monOpen.dispatchEvent(new Event('change', { bubbles: true }));
    await flush();

    saveButton(container).click();
    await flush();

    expect(saves().length, 'no save is sent with a cleared time').toBe(0);
  });

  it('BR6: the tablet name input holds at most 60 characters', async () => {
    const api = new ApiClient();
    const container = document.createElement('div');
    document.body.appendChild(container);
    disposers.push(render(() => <BranchTablets branchId={B01} branchCode="SILOM" api={api} />, container));
    await flush();

    const nameInput = container.querySelector<HTMLInputElement>('input[name="tabletName"]');
    expect(nameInput).not.toBeNull();
    expect(nameInput!.maxLength).toBe(60);
  });

  it('BR7: a 400 validationFailed at field timezone shows a worded line at the timezone select', async () => {
    saveRefusalField = 'timezone';
    const container = mountScreen();
    await flush();

    saveButton(container).click();
    await flush();
    expect(saves().length).toBe(1);

    const select = container.querySelector<HTMLSelectElement>('select[name="timezone"]');
    expect(select, 'timezone is a <select>').not.toBeNull();
    const field = select!.closest('.branches-field')!;
    const line = field.querySelector('[data-field-error="timezone"]');
    expect(line, 'a line at the timezone select').not.toBeNull();
    expect(line!.textContent).toBe(sharedTh['input.invalid'].replace('{field}', th.timezone));
    expect(container.textContent).not.toContain(SERVER_TEXT);
  });

  it("BR8: a 400 validationFailed at field hours.mon shows a worded line in Monday's row", async () => {
    saveRefusalField = 'hours.mon';
    const container = mountScreen();
    await flush();

    saveButton(container).click();
    await flush();
    expect(saves().length).toBe(1);

    const line = hoursRow(container, th.dayMon).querySelector('[data-field-error="hours.mon"]');
    expect(line, "a line inside Monday's row").not.toBeNull();
    expect(line!.textContent).toContain(th.dayMon);
    expect(line!.textContent!.endsWith(sharedTh['input.invalid'].replace('{field}', ''))).toBe(true);
    expect(container.textContent).not.toContain(SERVER_TEXT);
  });
});
