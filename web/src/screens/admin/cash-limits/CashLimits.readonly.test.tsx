// #cash-limits (count-difference limit and drawer low-cash levels) read-only for anyone without the save permission
// (owner: "เจ้าของเป็น คนตั้งค่า ไว้ ให้ ฝ่ายการเงินปฏิบัติ ตาม", "ตามเสนอ รวมใบอ่านอย่างเดียวด้วย"; server B2 #6004:
// `cashLimitsRead` needs settingsRead, `cashLimitsSave` and the levels save stay CashLimitsManage).
// Written before the implementation by claude/01TxaRJG (FE sub-controller). Do not edit, delete or weaken these tests;
// add your own elsewhere.
//
// [required] by these tests:
// - Both parts edit only when `sharedSession().me()?.permissions` holds 'CashLimitsManage'. Otherwise:
//   `input[data-field="countDifferenceLimitThb"]` and `input[data-action="no-limit"]` are `disabled`; no
//   `button[data-action="save"]`; every `input[data-field="lowLevel"]` is `disabled` and still shows its level; no
//   `button[data-action="add-level"]`, `[data-action="remove-level"]` or `[data-action="save-levels"]`; one
//   `[data-testid="owner-set"]` on the screen shows `ownerSet` (th 'ตั้งค่าโดยเจ้าของ', en 'Set by the owner');
//   submitting either form sends nothing.
// - With the permission there is no `[data-testid="owner-set"]` and the controls are as today.
// - The switch follows `me` as it changes. The code holds no role name.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { sharedSession } from '../../../shared/session/session';
import { makeCashLimitsData, makeMe } from '../../../shared/testing/fixtures';
import { Toast } from '../parts/toast';
import { CashLimits } from './CashLimits';

const MANAGE = 'CashLimitsManage';
const READ = 'settingsRead';
const OWNER_SET_TH = 'ตั้งค่าโดยเจ้าของ';
const OWNER_SET_EN = 'Set by the owner';

class Server {
  puts = 0;
  fetch = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const u = new URL(String(input), 'http://xc.test');
    const method = (init?.method ?? 'GET').toUpperCase();
    if (method === 'GET' && u.pathname === '/api/v3/views/cash_limits') {
      return json(200, {
        version: 1,
        changed: true,
        data: makeCashLimitsData({
          branch: 'MAIN',
          countDifferenceLimitThb: '500.00',
          updatedBy: 'u-exec',
          updatedByName: 'Owner',
          updatedAt: '2026-11-03T01:00:00Z',
          drawerLowLevels: [
            { currency: 'USD', lowLevel: '1000.00' },
            { currency: 'THB', lowLevel: '20000.00' },
          ],
        }),
      });
    }
    if (method === 'PUT') {
      this.puts++;
      return json(500, {});
    }
    return json(200, { version: 1, changed: true, data: null });
  };
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

let server: Server;
let host: HTMLDivElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  setLang('th');
  server = new Server();
  vi.stubGlobal('fetch', vi.fn(server.fetch));
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

function signIn(permissions: string[]) {
  sharedSession().updateMe(makeMe({ permissions }));
}

async function mount() {
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-11-03', to: '2026-11-03' });
  const [shown] = createSignal(true);
  dispose = render(
    () => (
      <>
        <CashLimits branch={branch} period={period} shown={shown} />
        <Toast />
      </>
    ),
    host,
  );
  await vi.waitFor(() => expect(levels()).toHaveLength(2));
}

const limit = () => host.querySelector<HTMLInputElement>('input[data-field="countDifferenceLimitThb"]');
const noLimit = () => host.querySelector<HTMLInputElement>('input[data-action="no-limit"]');
const levels = () => [...host.querySelectorAll<HTMLInputElement>('input[data-field="lowLevel"]')];
const action = (name: string) => host.querySelectorAll(`[data-action="${name}"]`);
const note = () => host.querySelectorAll('[data-testid="owner-set"]');
const settle = () => new Promise((r) => setTimeout(r, 0));

describe('#cash-limits read-only without CashLimitsManage', () => {
  it('RL1 a reader sees the limit and the levels in disabled fields, no edit buttons, and one owner note (TH and EN)', async () => {
    signIn([READ]);
    await mount();
    expect(limit()!.disabled).toBe(true);
    expect(noLimit()!.disabled).toBe(true);
    expect(action('save')).toHaveLength(0);
    expect(levels().map((i) => i.value)).toEqual(['1000.00', '20000.00']);
    expect(levels().every((i) => i.disabled), 'every level field is disabled').toBe(true);
    expect(action('add-level')).toHaveLength(0);
    expect(action('remove-level')).toHaveLength(0);
    expect(action('save-levels')).toHaveLength(0);
    expect(note()).toHaveLength(1);
    expect(note()[0]!.textContent).toBe(OWNER_SET_TH);
    setLang('en');
    await settle();
    expect(note()[0]!.textContent).toBe(OWNER_SET_EN);
  });

  it('RL2 a reader submitting either form sends nothing', async () => {
    signIn([READ]);
    await mount();
    const forms = [...host.querySelectorAll('form')];
    expect(forms.length).toBe(2);
    for (const f of forms) f.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();
    await settle();
    expect(server.puts).toBe(0);
  });

  it('RL3 a saver sees no owner note and the controls as today', async () => {
    signIn([READ, MANAGE]);
    await mount();
    expect(note()).toHaveLength(0);
    expect(limit()!.disabled).toBe(false);
    expect(noLimit()!.disabled).toBe(false);
    expect(action('save')).toHaveLength(1);
    expect(levels().every((i) => !i.disabled)).toBe(true);
    expect(action('add-level')).toHaveLength(1);
    expect(action('remove-level')).toHaveLength(2);
    expect(action('save-levels')).toHaveLength(1);
  });

  it('RL4 the switch follows me: a grant arriving later turns the controls on and the note off', async () => {
    signIn([READ]);
    await mount();
    expect(note()).toHaveLength(1);
    signIn([READ, MANAGE]);
    await vi.waitFor(() => expect(action('save-levels')).toHaveLength(1));
    expect(note()).toHaveLength(0);
    expect(limit()!.disabled).toBe(false);
  });
});
