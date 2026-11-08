// OL-6 wave B admin (audit #564 issuecomment-6702970101, rows 031-033): the reports screen's CSV action. Every tab shows
// its one CSV button in the heading of its own card, as the employee tab already does, and the toolbar above the tabs is
// gone, so the tab bar no longer jumps when a tab hides the toolbar's only button. `#export` stays the first tab's
// button (Reports.export-once.test.tsx:203). Written before the fix by claude/94389a6b. Do not edit, delete or weaken;
// add your own elsewhere.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { sharedSession } from '../../../shared/session/session';
import { Reports } from './Reports';
import { th } from './i18n';

const report = {
  period: { from: '2026-11-07', to: '2026-11-07' },
  stats: { received: '0.00', paid: '0.00', fees: '0.00', bankFees: '0.00' },
  byChannel: [],
  byCurrency: [],
  reference: { referenced: 0, unreferenced: 0, value: '0.00', spread: '0.00', percent: null },
  byEmployee: [],
  employeeTotals: { people: 0, completed: 0, received: '0.00', paid: '0.00', net: '0.00' },
};

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

async function mount(permissions: string[]): Promise<void> {
  sharedSession().updateMe?.({ id: 'owner', permissions } as never);
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) => {
      const view = new URL(String(input), 'http://xc.test').pathname.replace(/^\/api\/v3\/views\//, '');
      const data = view === 'admin_reports' ? report : view === 'admin_branches' ? { branches: [] } : [];
      return new Response(JSON.stringify({ version: 1, changed: true, data }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });
    }),
  );
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-11-07', to: '2026-11-07' });
  const [shown] = createSignal(true);
  dispose = render(() => <Reports branch={branch} period={period} shown={shown} />, host);
  for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
}

beforeEach(() => {
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

function shown(el: Element): boolean {
  for (let e: Element | null = el; e; e = e.parentElement) if ((e as HTMLElement).hidden) return false;
  return true;
}

const tabs = () => [...host.querySelectorAll<HTMLButtonElement>('[role="tab"]')];
// OL-7 row 61 (restated by claude/bbe9ce8b): every tab's CSV button reads th.exportCsv; downloadCsv is gone.
const CSV = new Set([th.exportCsv]);
const csvButtons = () =>
  [...host.querySelectorAll<HTMLButtonElement>('button')].filter((b) => CSV.has((b.textContent ?? '').trim()) && shown(b));

describe('reports CSV action', () => {
  it('RP-1 on every tab exactly one CSV button shows, in the heading of a card inside that tab', async () => {
    await mount(['ReportsExport']);
    const labels: string[] = [];
    for (const [i, tab] of tabs().entries()) {
      tab.click();
      await Promise.resolve();
      const buttons = csvButtons();
      expect(buttons, `tab ${i}`).toHaveLength(1);
      expect(buttons[0]!.closest(`#workspace-reports-${i} .card-heading`), `tab ${i}`).not.toBeNull();
      labels.push(buttons[0]!.textContent!.trim());
    }
    expect(labels).toEqual([th.exportCsv, th.exportCsv, th.exportCsv]);
  });

  it('RP-2 the toolbar above the tabs is gone', async () => {
    await mount(['ReportsExport']);
    expect(host.querySelector('.reports-toolbar')).toBeNull();
    expect(host.querySelector('.reports-screen')?.firstElementChild?.getAttribute('role')).toBe('tablist');
  });

  it('RP-3 #export is the first tab\'s button and ids stay unique', async () => {
    await mount(['ReportsExport']);
    expect(host.querySelectorAll('#export')).toHaveLength(1);
    expect(host.querySelector('#export')?.closest('#workspace-reports-0')).not.toBeNull();
    expect(host.querySelectorAll('#employee-csv')).toHaveLength(1);
  });

  it('RP-4 without the export permission no tab shows a CSV button', async () => {
    await mount([]);
    for (const tab of tabs()) {
      tab.click();
      await Promise.resolve();
      expect(csvButtons()).toHaveLength(0);
    }
  });
});
