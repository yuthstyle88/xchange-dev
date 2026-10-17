// #2496: the employee search is trimmed before it is read, keyed or filtered; an all-spaces
// search counts as no search. The input box keeps what the user typed.
// Render and mock setup copied from the neighbouring Reports.recover.test.tsx / worded.test.tsx.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { formatMoney } from '../../../shared/ui/MoneyText';
import { sharedSession } from '../../../shared/session/session';
import { Reports } from './Reports';

const ANAN = {
  staff: 'Anan',
  count: 3,
  completed: 3,
  pending: 0,
  cancelled: 0,
  received: '1234.00',
  paid: '200.00',
  net: '7771.00',
  fees: '10.00',
  bankFees: '2.00',
  methods: [],
  currencies: [],
};

function report(byEmployee: (typeof ANAN)[], people: number) {
  return {
    period: { from: '2026-10-05', to: '2026-10-06' },
    stats: { received: '1234.00', paid: '200.00', fees: '10.00', bankFees: '2.00' },
    byChannel: [],
    byCurrency: [],
    byEmployee,
    employeeTotals: people
      ? { people, completed: 3, received: '1234.00', paid: '200.00', net: '7771.00' }
      : { people: 0, completed: 0, received: '0.00', paid: '0.00', net: '0.00' },
    reference: { referenced: 0, unreferenced: 0, value: '0.00', spread: '0.00', percent: null },
  };
}

function json(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { 'content-type': 'application/json' },
  });
}

describe('Admin Reports: the employee search is trimmed (#2496)', () => {
  let root: HTMLElement;
  let dispose: (() => void) | undefined;
  let reportReads: URLSearchParams[] = [];
  let walks: URLSearchParams[] = [];
  let blobs: Blob[] = [];
  const originalCreate = URL.createObjectURL;
  const originalRevoke = URL.revokeObjectURL;

  beforeEach(() => {
    setLang('th');
    sharedSession().updateMe?.({ id: 'owner', permissions: ['ReportsExport'] } as any);
    reportReads = [];
    walks = [];
    blobs = [];
    URL.createObjectURL = ((b: Blob) => {
      blobs.push(b);
      return `blob:test/${blobs.length}`;
    }) as typeof URL.createObjectURL;
    URL.revokeObjectURL = (() => {}) as typeof URL.revokeObjectURL;
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});

    root = document.createElement('div');
    document.body.appendChild(root);

    vi.stubGlobal(
      'fetch',
      vi.fn((input: unknown) => {
        const u = new URL(String(input), 'http://desk.test');
        if (u.pathname === '/api/v3/views/admin_branches') {
          return Promise.resolve(
            json({ version: 1, changed: true, data: { branches: [{ id: 'MAIN', name: 'Silom' }] } }),
          );
        }
        if (u.pathname === '/api/v3/views/admin_reports') {
          reportReads.push(u.searchParams);
          // The server matches staffQ literally (admin_reports.rs:495-498): no trim.
          const q = u.searchParams.get('staffQ');
          const data =
            q === null || 'anan'.includes(q.toLowerCase()) ? report([ANAN], 1) : report([], 0);
          return Promise.resolve(json({ version: 1, changed: true, data }));
        }
        if (u.pathname === '/api/v3/views/admin_transactions') {
          walks.push(u.searchParams);
          return Promise.resolve(json({ version: 1, changed: true, data: [] }));
        }
        return Promise.resolve(json({ version: 1, changed: false }));
      }),
    );
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    root?.remove();
    URL.createObjectURL = originalCreate;
    URL.revokeObjectURL = originalRevoke;
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  function mount(): void {
    const [branch] = createSignal('MAIN');
    const [period] = createSignal({ from: '2026-10-05', to: '2026-10-06' });
    const [shown] = createSignal(true);
    dispose = render(() => <Reports branch={branch} period={period} shown={shown} />, root);
  }

  async function settle(): Promise<void> {
    for (let i = 0; i < 5; i++) await new Promise((r) => setTimeout(r, 10));
  }

  async function mounted(): Promise<void> {
    mount();
    await vi.waitFor(() => expect(reportReads.length).toBe(1));
    await settle();
  }

  function type(value: string): HTMLInputElement {
    const input = root.querySelector<HTMLInputElement>('#employee-search')!;
    input.value = value;
    input.dispatchEvent(new Event('input', { bubbles: true }));
    return input;
  }

  function employeeRows(): string[] {
    const panel = root.querySelector('#workspace-reports-2')!;
    return [...panel.querySelectorAll('tbody tr td:first-child')].map((td) => td.textContent ?? '');
  }

  it("Row 1: searching 'Anan ' (trailing space) sends staffQ=Anan and shows Anan's totals", async () => {
    await mounted();
    const input = type('Anan ');
    await vi.waitFor(() => expect(reportReads.length).toBe(2));
    await settle();

    expect(reportReads.every((q) => q.get('staffQ') !== 'Anan ')).toBe(true);
    expect(reportReads[reportReads.length - 1]!.get('staffQ')).toBe('Anan');
    const panel = root.querySelector('#workspace-reports-2')!;
    expect(panel.textContent).toContain(formatMoney('7771.00'));
    expect(employeeRows()).toEqual(['Anan']);
    expect(input.value, 'the box keeps what was typed').toBe('Anan ');
  });

  it("Row 2: a search of only spaces sends no staffQ, the same as an empty search", async () => {
    await mounted();
    type('   ');
    await settle();
    await settle();

    expect(reportReads.length).toBeGreaterThan(0);
    for (const q of reportReads) expect(q.has('staffQ')).toBe(false);
    expect(employeeRows()).toEqual(['Anan']);
  });

  it("Row 3: the Employee CSV with 'Anan ' filters on staffQ=Anan", async () => {
    await mounted();
    type('Anan ');
    await settle();
    root.querySelector<HTMLButtonElement>('#employee-csv')!.click();
    await vi.waitFor(() => expect(walks.length).toBeGreaterThan(0));
    await settle();

    expect(walks[0]!.get('staffQ')).toBe('Anan');
  });

  it('Edge: changing only trailing spaces does not start a new read', async () => {
    await mounted();
    type('Anan');
    await vi.waitFor(() => expect(reportReads.length).toBe(2));
    await settle();

    type('Anan  ');
    await settle();
    await settle();
    expect(reportReads.length).toBe(2);
  });
});
