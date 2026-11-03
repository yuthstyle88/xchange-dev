// #cash-policy read-only for anyone without the save permission (owner: "เจ้าของเป็น คนตั้งค่า ไว้ ให้ ฝ่ายการเงินปฏิบัติ ตาม",
// "ทำเหมือน ระบบ ธนาคาร ทั่วไป", "ตามเสนอ รวมใบอ่านอย่างเดียวด้วย"; server B2 #6004: `transferPolicyRead` needs settingsRead,
// `transferPolicySave` stays CashTransferPolicySet).
// Written before the implementation by claude/01TxaRJG (FE sub-controller). Do not edit, delete or weaken these tests;
// add your own elsewhere.
//
// [required] by these tests:
// - The screen edits only when `sharedSession().me()?.permissions` holds 'CashTransferPolicySet'. Otherwise:
//   every `input[data-currency]` is `disabled` and still shows the saved limit; there is no `select[name="addCurrency"]`
//   and no `button[data-action="save"]`; one `[data-testid="owner-set"]` shows `ownerSet` (th 'ตั้งค่าโดยเจ้าของ',
//   en 'Set by the owner'); submitting the form sends nothing.
// - With the permission there is no `[data-testid="owner-set"]` and the controls are as today.
// - The switch follows `me` as it changes (a grant arriving later turns the controls on). The code holds no role name.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { sharedSession } from '../../../shared/session/session';
import {
  makeAdminCashData,
  makeAdminCashStockRow,
  makeCashTransferPolicyData,
  makeMe,
} from '../../../shared/testing/fixtures';
import { Toast } from '../parts/toast';
import { CashPolicy } from './CashPolicy';

const SET = 'CashTransferPolicySet';
const READ = 'settingsRead';
const OWNER_SET_TH = 'ตั้งค่าโดยเจ้าของ';
const OWNER_SET_EN = 'Set by the owner';

class Server {
  policy: unknown = makeCashTransferPolicyData({ version: 'v1', limits: { USD: '500.00' } });
  puts = 0;

  fetch = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const u = new URL(String(input), 'http://xc.test');
    const method = (init?.method ?? 'GET').toUpperCase();
    if (method === 'GET' && u.pathname === '/api/v3/views/admin_cash') {
      return json(200, {
        version: 1,
        changed: true,
        data: makeAdminCashData({
          openShifts: [],
          stock: ['THB', 'USD', 'EUR'].map((code) => makeAdminCashStockRow({ code })),
          stats: null,
          currencies: ['THB', 'EUR', 'GBP', 'JPY', 'USD'],
          lastCounted: null,
        }),
      });
    }
    if (method === 'GET' && u.pathname === '/api/v3/views/cash_transfer_policy') {
      return json(200, { changed: true, versions: {}, data: this.policy });
    }
    if (method === 'PUT') {
      this.puts++;
      return json(200, this.policy);
    }
    return json(404, { error: { code: 'notFound', message: `unexpected ${method} ${u.pathname}` } });
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
        <CashPolicy branch={branch} period={period} shown={shown} />
        <Toast />
      </>
    ),
    host,
  );
  await vi.waitFor(() => expect(input('USD')?.value).toBe('500.00'));
}

const inputs = () => [...host.querySelectorAll<HTMLInputElement>('input[data-currency]')];
const input = (cur: string) => host.querySelector<HTMLInputElement>(`input[data-currency="${cur}"]`);
const note = () => host.querySelectorAll('[data-testid="owner-set"]');
const settle = () => new Promise((r) => setTimeout(r, 0));

describe('#cash-policy read-only without CashTransferPolicySet', () => {
  it('RO1 a reader sees the saved limits in disabled fields, no add select, no save, and the owner note (TH and EN)', async () => {
    signIn([READ]);
    await mount();
    expect(inputs().map((i) => i.dataset.currency)).toEqual(['THB', 'USD', 'EUR']);
    expect(inputs().every((i) => i.disabled), 'every limit field is disabled').toBe(true);
    expect(host.querySelector('select[name="addCurrency"]')).toBeNull();
    expect(host.querySelector('button[data-action="save"]')).toBeNull();
    expect(note()).toHaveLength(1);
    expect(note()[0]!.textContent).toBe(OWNER_SET_TH);
    setLang('en');
    await settle();
    expect(note()[0]!.textContent).toBe(OWNER_SET_EN);
  });

  it('RO2 a reader submitting the form sends nothing', async () => {
    signIn([READ]);
    await mount();
    host.querySelector('form')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();
    await settle();
    expect(server.puts).toBe(0);
  });

  it('RO3 a saver sees no owner note and the controls as today', async () => {
    signIn([READ, SET]);
    await mount();
    expect(note()).toHaveLength(0);
    expect(inputs().every((i) => !i.disabled), 'every limit field is editable').toBe(true);
    expect(host.querySelector('select[name="addCurrency"]')).not.toBeNull();
    expect(host.querySelector('button[data-action="save"]')).not.toBeNull();
  });

  it('RO4 the switch follows me: a grant arriving later turns the controls on and the note off', async () => {
    signIn([READ]);
    await mount();
    expect(note()).toHaveLength(1);
    signIn([READ, SET]);
    await vi.waitFor(() => expect(host.querySelector('button[data-action="save"]')).not.toBeNull());
    expect(note()).toHaveLength(0);
    expect(input('USD')!.disabled).toBe(false);
  });
});
