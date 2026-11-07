// OL-7 device-events-names, the screen half (verifier row 98: "/devices audit table still shows truncated UUIDs";
// sub-controller claude/1cc51615's decision, /tmp/ol7/ORDER-claude-19-b.md item 3). The device_events view now answers
// `actorName` with each row: the user's name when `actor` is a user id, else null
// (api/tests/v3_device_events_actor_name_test.rs). The event table's actor cell shows actorName when there is one and
// the stored actor otherwise; its `title` always holds the stored actor, so the id stays one hover away.
// Written before the implementation by claude/94389a6b. Do not edit, delete or weaken; add tests in a file of your own.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { ApiClient, PageParams, ViewAnswer, ViewParams } from '../../shared/api/client';
import {
  makeDeviceBranch,
  makeDeviceEventRow,
  makeDevicesOverview,
} from '../../shared/testing/fixtures';
import { setLang } from '../../shared/i18n';
import { DevicesScreen } from './DevicesScreen';
import { th } from './i18n';

type Api = Pick<ApiClient, 'view' | 'post' | 'page'>;

const ANNA_ID = '5fe9d9ce-5400-4000-8000-000000000001';
const AGENT = 'agent:7b1c2a90-0000-4000-8000-000000000001';

/** A row as the view answers it now (restated by the sub-controller: the no-inline-dto guard forbids naming or casting the generated type). */
function eventRow(seq: string, actor: string, actorName: string | null) {
  return makeDeviceEventRow({ seq, kind: 'PRINT_RECEIPT', ref: `TX-${seq}`, actor, actorName, detail: '{}' });
}

const EVENTS = [eventRow('3', ANNA_ID, 'Anna Teller'), eventRow('2', AGENT, null), eventRow('1', 'ลูกค้า (ตู้)', null)];

function api(): Api {
  const data: Record<string, unknown> = {
    devices: makeDevicesOverview({
      branchId: 'MAIN',
      branches: [makeDeviceBranch({ id: 'MAIN', name: 'สำนักงานใหญ่' })],
      printerOnline: true,
      printerKind: 'SIMULATED',
      printerModel: 'NEOCAL NTP-8220BT',
      hasDrawer: null,
      drawerAgentActive: false,
      balanceThb: '0.00',
      balanceUsd: '0.00',
      completedCount: 0,
    }),
    device_jobs: [],
    device_events: EVENTS,
    device_transactions: [],
  };
  return {
    view<T>(name: string, _q?: ViewParams): Promise<ViewAnswer<T>> {
      if (!(name in data)) return Promise.reject(new Error(`unknown view ${name}`));
      return Promise.resolve({ version: 1, changed: true, data: structuredClone(data[name]) as T });
    },
    page<R>(name: string, q?: PageParams): Promise<R[]> {
      const rows = q?.before === undefined ? ((data[name] as unknown[] | undefined) ?? []) : [];
      return Promise.resolve(structuredClone(rows) as R[]);
    },
    post<B, R>(_path: string, _body: B): Promise<R> {
      return Promise.resolve({ saved: true } as unknown as R);
    },
  };
}

let dispose: (() => void) | undefined;
let host: HTMLDivElement;

beforeEach(() => {
  setLang('th');
  vi.stubGlobal('fetch', vi.fn(() => Promise.reject(new Error('offline in tests'))));
  vi.spyOn(window.history, 'replaceState').mockImplementation(() => {});
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  setLang('th');
});

async function mount(): Promise<void> {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <DevicesScreen api={api()} branch="MAIN" download={() => {}} />, host);
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
}

/** The actor cell of each event row, newest first. */
function actorCells(): HTMLElement[] {
  return [...host.querySelectorAll<HTMLElement>(`.events-table tbody td[data-label="${th.colActor}"]`)];
}

describe('device log actor cell', () => {
  it('DA-1 a user line shows the user name, and keeps the stored id as its title', async () => {
    await mount();
    const cell = actorCells()[0]!;
    expect(cell.textContent?.trim()).toBe('Anna Teller');
    expect(cell.textContent).not.toContain(ANNA_ID);
    expect(cell.querySelector('[title]')?.getAttribute('title')).toBe(ANNA_ID);
  });

  it('DA-2 a line with no name shows the stored actor, as before', async () => {
    await mount();
    const [, agent, kiosk] = actorCells();
    expect(agent!.textContent?.trim()).toBe(AGENT);
    expect(agent!.querySelector('[title]')?.getAttribute('title')).toBe(AGENT);
    expect(kiosk!.textContent?.trim()).toBe('ลูกค้า (ตู้)');
  });

  it('DA-4 a name is not cut like an id: its span is not .dev-id (12ch, ellipsis); a bare actor still is', async () => {
    await mount();
    const [user, agent] = actorCells();
    expect(user!.querySelector('.dev-id')).toBeNull();
    expect(agent!.querySelector('.dev-id')?.getAttribute('title')).toBe(AGENT);
  });

  it('DA-3 three lines, three actor cells (no row dropped or added)', async () => {
    await mount();
    expect(actorCells()).toHaveLength(3);
  });
});
