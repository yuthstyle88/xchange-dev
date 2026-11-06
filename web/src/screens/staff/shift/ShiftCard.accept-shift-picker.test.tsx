// Up-front tests for CS-15m (docs/design/configurable-shifts.md §2.3 "Handover", CS-19 row H1): accepting a
// handover opens the receiver's shift under a template, so ShiftCard mounts the existing ShiftPicker in the
// "handovers to me" section. With two covering shifts the accept waits for a pick; the POST to
// /api/handovers/{id}/accept carries `shiftTemplateId`. Row M5 (a, b, c).
// Written by the ticket author claude/35271802 — the implementer must not edit, delete or weaken these.
//
// Selectors read here, all of which already exist:
//   - `shift-picker`, `shift-choice-<templateId>` (label; its radio input inside), `outside-shift-hours`
//     — rendered by ShiftPicker.tsx; mount ONE picker for the section (not one per row: the radios share
//     name="shift-choice"), pass ShiftCard's own ApiClient as `api`;
//   - `[data-handover-id="<id>"]`, `shift-accept`, `shift-reject` — ShiftCard.tsx today.
// No new data-testid is needed. The accept body type is a local one in ShiftCard.tsx, e.g.
// `type HandoverAcceptBody = CashRecordInput & { shiftTemplateId?: string }` (the server's HandoverAcceptInput,
// api/src/presentation/http/v3/commands/cash/handovers.rs, derives no TS type).
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { JSX } from 'solid-js';
import { setLang } from '../../../shared/i18n';
import {
  makeHandoverPendingRow,
  makeOpenShiftChoice,
  makeOpenShiftChoices,
  makeStaffFloorView,
  makeStaffMeView,
} from '../../../shared/testing/fixtures';
import type { StaffCard, StaffCtx } from '../cards';
import { warmModules } from '../../../shared/testing/warm-modules';

const cardMods = import.meta.glob<{ card: StaffCard }>('./card.ts');

async function loadCard(): Promise<StaffCard> {
  const l = cardMods['./card.ts'];
  expect(l, 'web/src/screens/staff/shift/card.ts exists').toBeDefined();
  return (await l!()).card;
}

const BRANCH = 'MAIN';
const ME = '00000000-0000-4000-8000-0000000000a1';
const H_MINE = 'd0000000-0000-4000-8000-000000000001';
const ACCEPT = `/api/handovers/${H_MINE}/accept`;
const TPL_A = 'tpl-a';
const TPL_B = 'tpl-b';
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

const HANDOVERS = [
  makeHandoverPendingRow({
    id: H_MINE,
    fromShiftId: '33333333-3333-4333-8333-333333333333',
    countId: '44444444-4444-4444-8444-444444444444',
    receiverId: ME,
    status: 'PENDING',
    createdAt: '2026-11-05T03:00:00Z',
  }),
];

type Sent = { url: string; method: string; body: Record<string, unknown> };

let sent: Sent[];
let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

function stub(choices: unknown) {
  const all: Record<string, unknown> = {
    cash_counts: [],
    handovers_pending: HANDOVERS,
    my_shift: null,
    denominations: { rows: [] },
    handover_receivers: { rows: [] },
    open_shift_choices: choices,
  };
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const path = url.split('?')[0]!;
      const method = (init?.method ?? 'GET').toUpperCase();
      if (method === 'GET') {
        const name = path.startsWith('/api/v3/views/') ? path.slice('/api/v3/views/'.length) : '';
        if (name in all) return json(200, { version: 1, changed: true, data: all[name] });
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }
      sent.push({ url: path, method, body: JSON.parse(String(init?.body ?? '{}')) });
      return json(200, { id: H_MINE, status: 'ACCEPTED' });
    }),
  );
}

async function settle() {
  for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
}

async function mount() {
  const card = await loadCard();
  const ctx: StaffCtx = {
    me: makeStaffMeView({ userId: ME, name: 'Ploy', roles: [], branchId: BRANCH, branchName: 'XC-B', permissions: ['ShiftOperate'] }),
    floor: makeStaffFloorView({
      phase: 'OPEN',
      hours: null,
      myClockIn: null,
      myShift: null,
      cutoff: null,
      target: null,
      safe: null,
    }),
    refresh: vi.fn(async () => {}),
  };
  const C = card.Component as (p: { ctx: StaffCtx }) => JSX.Element;
  dispose = render(() => <C ctx={ctx} />, host);
  await settle();
}

const q = <T extends Element = HTMLElement>(sel: string) => host.querySelector<T>(sel);
const tid = (id: string) => `[data-testid="${id}"]`;
const accept = () => q<HTMLButtonElement>(`[data-handover-id="${H_MINE}"] ${tid('shift-accept')}`);
const reject = () => q<HTMLButtonElement>(`[data-handover-id="${H_MINE}"] ${tid('shift-reject')}`);
const radio = (templateId: string) =>
  q<HTMLInputElement>(`${tid(`shift-choice-${templateId}`)} input[type="radio"]`);
const acceptPosts = () => sent.filter((s) => s.method === 'POST' && s.url === ACCEPT);

beforeAll(
  () =>
    warmModules(
      () => import('./card'),
      () => import('../main'),
    ),
  60_000,
);

beforeEach(() => {
  sent = [];
  setLang('th');
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.unstubAllGlobals();
});

describe('CS-15m ShiftCard handover accept chooses the shift to open', () => {
  it('M5a two covering shifts: the picker asks, accept stays disabled until a pick, then the POST carries it', async () => {
    stub(makeOpenShiftChoices({ choices: [CHOICE_A, CHOICE_B], preselect: null }));
    await mount();
    await vi.waitFor(() => expect(accept()).not.toBeNull());

    await vi.waitFor(() => expect(q(tid('shift-picker')), 'shift-picker in the handovers-to-me section').not.toBeNull());
    expect(radio(TPL_A)!.checked).toBe(false);
    expect(radio(TPL_B)!.checked).toBe(false);
    expect(accept()!.disabled, 'accept waits for a shift to be chosen').toBe(true);

    accept()!.click();
    await settle();
    expect(acceptPosts(), 'an accept before a shift is chosen sends nothing').toHaveLength(0);

    radio(TPL_B)!.click();
    await settle();
    expect(accept()!.disabled).toBe(false);

    accept()!.click();
    await settle();
    expect(acceptPosts()).toHaveLength(1);
    expect(acceptPosts()[0]!.body).toMatchObject({ id: H_MINE, branch: BRANCH, shiftTemplateId: TPL_B });
  });

  it('M5b one covering shift: accept needs no pick and the POST carries that shiftTemplateId', async () => {
    stub(makeOpenShiftChoices({ choices: [CHOICE_A], preselect: TPL_A }));
    await mount();
    await vi.waitFor(() => {
      expect(accept()).not.toBeNull();
      expect(accept()!.disabled).toBe(false);
    });
    await settle();

    accept()!.click();
    await settle();
    expect(acceptPosts()).toHaveLength(1);
    expect(acceptPosts()[0]!.body).toMatchObject({ id: H_MINE, branch: BRANCH, shiftTemplateId: TPL_A });
  });

  it('M5c no covering shift: outside-shift-hours shows, accept disabled and sends nothing, reject still usable', async () => {
    stub(makeOpenShiftChoices({ choices: [], preselect: null }));
    await mount();
    await vi.waitFor(() => expect(accept()).not.toBeNull());

    await vi.waitFor(() => expect(q(tid('outside-shift-hours')), 'outside-shift-hours in the handovers-to-me section').not.toBeNull());
    expect(accept()!.disabled).toBe(true);
    expect(reject()!.disabled).toBe(false);

    accept()!.click();
    await settle();
    expect(acceptPosts()).toHaveLength(0);
  });
});
