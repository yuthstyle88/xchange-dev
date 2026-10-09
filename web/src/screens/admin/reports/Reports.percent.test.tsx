// Ticket #526: written before the fix by the ticket author (claude/e5ab6c). Do not edit,
// delete or weaken; add tests in a file of your own.
//
// The reference line of the currencies tab prints the percent the server sends, and nothing
// when the server sends none. v2 prints `0.00%` when exchanges have a reference rate but their
// reference value sums to 0 (`xchange-v2-source@4d0bdbb` `src/online/lib/admin-html.ts`,
// `referenceSummary`: `(reference ? spread/reference*100 : 0).toFixed(2)`). After #526 that
// figure comes from the server (`admin_reports` answers `percent: "0.00"` in that case), so the
// screen no longer makes one up (`Reports.tsx:449`, `?? '0.00'`).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { formatMoney } from '../../../shared/ui/MoneyText';
import type { AdminMenu, AdminMenuProps } from '../menus';

interface MenuModule {
  menu: AdminMenu & { Component: (props: AdminMenuProps) => JSX.Element };
}

const menuFiles = import.meta.glob<MenuModule>('./menu.ts', { eager: true });

const NO_REFERENCE = 'ยังไม่มีเรทอ้างอิง';

interface Reference {
  referenced: number;
  unreferenced: number;
  value: string;
  spread: string;
  percent: string | null;
}

function channels() {
  return ['Cash', 'Bank Transfer', 'Credit Card', 'Alipay', 'PromptPay'].map((method) => ({
    method,
    count: 0,
    received: '0.00',
    paid: '0.00',
    net: '0.00',
  }));
}

function answerWith(reference: Reference) {
  return {
    period: { from: '2026-10-05', to: '2026-10-06' },
    stats: { received: '0.00', paid: '0.30', fees: '0.00', bankFees: '0.00' },
    byChannel: channels(),
    byCurrency: [{ currency: 'USD', buyAmount: '0.01', sellAmount: '0.00', principal: '0.30', fees: '0.00' }],
    reference,
    byEmployee: [],
    employeeTotals: { people: 0, completed: 0, received: '0.00', paid: '0.00', net: '0.00' },
  };
}

function reply(status: number, body: unknown): Promise<Response> {
  return Promise.resolve(
    new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } }),
  );
}

let reference: Reference;
let root: HTMLElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  setLang('th');
  vi.stubGlobal(
    'fetch',
    vi.fn((input: unknown) => {
      const u = new URL(String(input), 'http://desk.test');
      const view = u.pathname.slice('/api/v3/views/'.length);
      switch (view) {
        case 'admin_reports':
          return reply(200, { version: 1, changed: true, data: answerWith(reference) });
        case 'admin_transactions':
          return reply(200, { version: 1, changed: true, data: [] });
        case 'admin_branches':
          return reply(200, { version: 1, changed: true, data: { branches: [{ id: 'MAIN', name: 'MAIN' }] } });
        case 'me':
          return reply(200, { version: 1, changed: true, data: { kind: 'owner', id: 'owner', role: 'OWNER', branch: null } });
        default:
          return reply(200, { version: 1, changed: false });
      }
    }),
  );
  location.hash = '#reports';
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  root?.remove();
  vi.unstubAllGlobals();
  location.hash = '';
  setLang('th');
});

async function referenceLine(): Promise<string> {
  const m = menuFiles['./menu.ts'];
  expect(m, 'web/src/screens/admin/reports/menu.ts exists').toBeDefined();
  const C = m!.menu.Component;
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-10-05', to: '2026-10-06' });
  const [shown] = createSignal(true);
  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(() => <C branch={branch} period={period} shown={shown} />, root);
  const panel = (): HTMLElement | null => root.querySelector('#workspace-reports-1');
  // the answer has landed once the panel shows this answer's counts (every case below has a
  // non-zero count, and before the answer both read 0)
  await vi.waitFor(() => {
    const text = (panel()?.textContent ?? '').replace(/\s+/g, ' ');
    expect(text, 'the answer is shown').toContain(
      `${reference.referenced} รายการมีเรทอ้างอิง · ${reference.unreferenced} รายการยังไม่ระบุ`,
    );
    expect(reference.referenced + reference.unreferenced, 'a case with a count').toBeGreaterThan(0);
  });
  const text = (panel()!.textContent ?? '').replace(/\s+/g, ' ');
  const at = text.indexOf('ส่วนต่างเทียบเรทอ้างอิง');
  expect(at, `the reference line is in the panel: ${text}`).toBeGreaterThanOrEqual(0);
  return text.slice(at, at + 120);
}

describe('#526: the reference percent is the server\'s, never made up by the screen', () => {
  it('referenced > 0 and percent "0.00" (v2: reference sums to 0): prints the server\'s 0.00%', async () => {
    reference = { referenced: 1, unreferenced: 0, value: '0.00', spread: '-0.30', percent: '0.00' };
    const line = await referenceLine();
    expect(line).toContain(`${formatMoney('-0.30')} THB (0.00%)`);
  });

  it('referenced > 0 and percent "1.65": prints 1.65% verbatim', async () => {
    reference = { referenced: 2, unreferenced: 3, value: '4131.66', spread: '68.34', percent: '1.65' };
    const line = await referenceLine();
    expect(line).toContain(`${formatMoney('68.34')} THB (1.65%)`);
  });

  it('referenced > 0 and percent null: prints the spread and no percent at all', async () => {
    reference = { referenced: 1, unreferenced: 0, value: '0.00', spread: '-0.30', percent: null };
    const line = await referenceLine();
    expect(line).toContain(`${formatMoney('-0.30')} THB`);
    expect(line, 'no percent the server did not send').not.toContain('%');
  });

  it('referenced 0: prints ยังไม่มีเรทอ้างอิง', async () => {
    reference = { referenced: 0, unreferenced: 2, value: '0.00', spread: '0.00', percent: null };
    const line = await referenceLine();
    expect(line).toContain(NO_REFERENCE);
    expect(line).not.toContain('%');
  });
});
