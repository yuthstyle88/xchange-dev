// Up-front tests for CS-15m (docs/design/configurable-shifts.md, CS-7 OPEN count with shiftTemplateId): the
// existing ShiftPicker is mounted in CountForm for an OPEN count, so a teller can choose the shift when two
// templates cover now (the server refuses `shiftNotChosen` without one) and the POST to /api/cash-counts
// carries `shiftTemplateId`. Rows M1–M4.
// Written by the ticket author claude/35271802 — the implementer must not edit, delete or weaken these.
//
// Selectors read here, all of which already exist:
//   - `shift-picker`, `shift-choice-<templateId>` (label; its radio input inside), `outside-shift-hours`
//     — rendered by ShiftPicker.tsx, mount it as is (pass CountForm's own ApiClient as `api`). Render it next to
//     the <form data-testid="shift-count-form">, not inside it: shift.test.tsx S9 holds that every input inside
//     the count form is a face-value quantity (the currency search box is its one exception);
//   - `shift-count-form`, `shift-count-save`, `input[data-denomination-id]` — CountForm.tsx today.
// No new data-testid is needed.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import {
  makeDenominationRow,
  makeMyShift,
  makeMyShiftBalance,
  makeOpenShiftChoice,
  makeOpenShiftChoices,
  makeStaffFloorView,
  makeStaffMeView,
} from '../../../shared/testing/fixtures';
import type { StaffCtx } from '../cards';
import { CountForm } from './CountForm';

const BRANCH = 'MAIN';
const ME = 'u-teller';
const SHIFT = 'shift-1';
const TPL_A = 'tpl-a';
const TPL_B = 'tpl-b';
const DENOMS = [
  makeDenominationRow({ id: 11, currency: 'THB', faceValue: '1000.0000', groupCode: 'NOTE' }),
  makeDenominationRow({ id: 12, currency: 'THB', faceValue: '100.0000', groupCode: 'NOTE' }),
];
const CHOICE_A = makeOpenShiftChoice({
  templateId: TPL_A,
  name: 'Morning',
  startsAt: '2026-11-04T23:00:00Z',
  endsAt: '2026-11-05T07:00:00Z',
});
const CHOICE_B = makeOpenShiftChoice({
  templateId: TPL_B,
  name: 'Overlap',
  startsAt: '2026-11-05T03:00:00Z',
  endsAt: '2026-11-05T11:00:00Z',
});

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let views: string[] = [];
let viewQueries: Array<{ view: string; query: string }> = [];
let posts: Array<{ path: string; body: Record<string, unknown> }> = [];
let choices: unknown = makeOpenShiftChoices({ choices: [CHOICE_A], preselect: TPL_A });

function stub() {
  views = [];
  viewQueries = [];
  posts = [];
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const [path, query = ''] = url.split('?') as [string, string?];
      const method = (init?.method ?? 'GET').toUpperCase();
      if (method === 'GET') {
        const view = path.startsWith('/api/v3/views/') ? path.slice('/api/v3/views/'.length) : '';
        views.push(view);
        viewQueries.push({ view, query });
        if (view === 'denominations') return json(200, { version: 1, changed: true, data: { rows: DENOMS } });
        if (view === 'open_shift_choices') return json(200, { version: 1, changed: true, data: choices });
        if (view === 'my_shift') {
          return json(200, {
            version: 1,
            changed: true,
            data: makeMyShift({
              id: SHIFT,
              balances: [makeMyShiftBalance({ code: 'THB', opening: '0', expected: '5000.00' })],
            }),
          });
        }
        return json(404, { error: { code: 'notFound', message: 'x' } });
      }
      posts.push({ path, body: JSON.parse(String(init?.body ?? '{}')) });
      return json(200, { id: 'cc-1' });
    }),
  );
}

function mount(kind: 'OPEN' | 'CLOSE') {
  const onSaved = vi.fn(async () => {});
  const onError = vi.fn();
  const [busy, setBusy] = createSignal(false);
  const [denoms] = createSignal(DENOMS);
  const ctx: StaffCtx = {
    me: makeStaffMeView({ userId: ME, name: 'Somchai', roles: [], branchId: BRANCH, branchName: 'XC-B', permissions: ['ShiftOperate'] }),
    floor: makeStaffFloorView({
      phase: 'OPEN',
      hours: null,
      myClockIn: '2026-11-05T01:00:00Z',
      myShift: kind === 'CLOSE' ? { id: SHIFT, openedAt: '2026-11-05T01:05:00Z' } : null,
      cutoff: null,
      target: null,
      safe: null,
    }),
    refresh: vi.fn(async () => {}),
  };
  dispose = render(
    () => (
      <CountForm
        ctx={ctx}
        kind={kind}
        denominations={denoms}
        busy={busy}
        setBusy={setBusy}
        onSaved={onSaved}
        onCancel={vi.fn()}
        onError={onError}
      />
    ),
    host,
  );
  return { onSaved, onError };
}

async function settle() {
  for (let i = 0; i < 8; i++) await new Promise((r) => setTimeout(r, 0));
}

const q = <T extends Element = HTMLElement>(sel: string) => host.querySelector<T>(sel);
const tid = (id: string) => `[data-testid="${id}"]`;
const save = () => q<HTMLButtonElement>(tid('shift-count-save'))!;
const radio = (templateId: string) =>
  q<HTMLInputElement>(`${tid(`shift-choice-${templateId}`)} input[type="radio"]`);
function type(id: number, v: string) {
  const el = q<HTMLInputElement>(`input[data-denomination-id="${id}"]`)!;
  el.value = v;
  el.dispatchEvent(new Event('input', { bubbles: true }));
}
const submit = () =>
  q<HTMLFormElement>(tid('shift-count-form'))!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));

describe('CS-15m CountForm mounts ShiftPicker for an OPEN count', () => {
  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
    setLang('th');
    choices = makeOpenShiftChoices({ choices: [CHOICE_A], preselect: TPL_A });
    stub();
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.unstubAllGlobals();
  });

  it('M1 one covering shift: shown preselected, Save enabled, the POST carries its shiftTemplateId', async () => {
    choices = makeOpenShiftChoices({ choices: [CHOICE_A], preselect: TPL_A });
    mount('OPEN');
    await settle();

    await vi.waitFor(() => expect(q(tid('shift-picker')), 'shift-picker with the OPEN count form').not.toBeNull());
    expect(viewQueries.find((v) => v.view === 'open_shift_choices')?.query).toContain(`branch=${BRANCH}`);
    expect(radio(TPL_A), `radio inside shift-choice-${TPL_A}`).not.toBeNull();
    expect(radio(TPL_A)!.checked).toBe(true);
    expect(save().disabled).toBe(false);

    type(11, '3');
    submit();
    await settle();
    expect(posts).toHaveLength(1);
    expect(posts[0]!.path).toBe('/api/cash-counts');
    expect(posts[0]!.body).toMatchObject({
      branch: BRANCH,
      kind: 'OPEN',
      shiftId: null,
      shiftTemplateId: TPL_A,
      lines: [{ currency: 'THB', denominationId: 11, qty: '3' }],
    });
  });

  it('M2 two covering shifts: none chosen and Save disabled; after a pick Save is enabled and the POST carries the pick', async () => {
    choices = makeOpenShiftChoices({ choices: [CHOICE_A, CHOICE_B], preselect: null });
    mount('OPEN');
    await settle();

    await vi.waitFor(() => expect(q(tid('shift-picker')), 'shift-picker with the OPEN count form').not.toBeNull());
    expect(radio(TPL_A)!.checked).toBe(false);
    expect(radio(TPL_B)!.checked).toBe(false);
    expect(save().disabled, 'Save waits for a shift to be chosen').toBe(true);

    type(11, '3');
    submit();
    await settle();
    expect(posts, 'a submit before a shift is chosen sends nothing').toHaveLength(0);

    radio(TPL_B)!.click();
    await settle();
    expect(radio(TPL_B)!.checked).toBe(true);
    expect(save().disabled).toBe(false);

    submit();
    await settle();
    expect(posts).toHaveLength(1);
    expect(posts[0]!.path).toBe('/api/cash-counts');
    expect(posts[0]!.body).toMatchObject({ kind: 'OPEN', shiftId: null, shiftTemplateId: TPL_B });
  });

  it('M3 no covering shift: outside-shift-hours shows, Save disabled, nothing is posted', async () => {
    choices = makeOpenShiftChoices({ choices: [], preselect: null });
    mount('OPEN');
    await settle();

    await vi.waitFor(() => expect(q(tid('outside-shift-hours')), 'outside-shift-hours with the OPEN count form').not.toBeNull());
    expect(save().disabled).toBe(true);

    type(11, '3');
    submit();
    await settle();
    expect(posts).toHaveLength(0);
  });

  // M4 is a no-regression row: it passes on today's code and must keep passing once the picker is mounted.
  it('M4 CLOSE count: no picker, no open_shift_choices read, the body has no shiftTemplateId, Save as before', async () => {
    // Two choices on offer: a picker wrongly mounted for CLOSE would also disable Save here.
    choices = makeOpenShiftChoices({ choices: [CHOICE_A, CHOICE_B], preselect: null });
    mount('CLOSE');
    await settle();

    expect(q(tid('shift-picker'))).toBeNull();
    expect(q(tid('outside-shift-hours'))).toBeNull();
    expect(views).not.toContain('open_shift_choices');
    expect(save().disabled).toBe(false);

    type(12, '2');
    submit();
    await settle();
    expect(posts).toHaveLength(1);
    expect(posts[0]!.body).toMatchObject({ kind: 'CLOSE', shiftId: SHIFT });
    expect(posts[0]!.body).not.toHaveProperty('shiftTemplateId');
  });
});
