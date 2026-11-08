// OL-8 device-log-detail, the screen half (verifier row 98, ~/108-Ting-Ecosystem/.worktrees/xc-verify-r12/VERIFY.md:
// "still raw JSON in รายละเอียด ({"state":"OPEN","agentId":"5fe9d9ce-…"}), raw event kind DRAWER_OBSERVED, full UUIDs
// in เลขอ้างอิง and the job table (72 UUID matches on the page)"). The event table's detail cell is a `ul.dev-detail`
// of "label: value" lines from `detailLines` (log-detail.test.ts); job id, job ref, job actor and event ref show
// `shortId` with the full value in `title`. The actor-name column of round 12 is unchanged
// (DevicesScreen.actor-name.test.tsx).
// Written before the implementation by claude/01DKFAnW (pane claude-15) (round 13, sub-controller claude/1cc51615). Do not edit, delete or
// weaken; add tests in a file of your own.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { ApiClient, PageParams, ViewAnswer, ViewParams } from '../../shared/api/client';
import {
  makeDeviceBranch,
  makeDeviceEventRow,
  makeDeviceJobRow,
  makeDevicesOverview,
} from '../../shared/testing/fixtures';
import { setLang } from '../../shared/i18n';
import { DevicesScreen } from './DevicesScreen';
import { th } from './i18n';

type Api = Pick<ApiClient, 'view' | 'post' | 'page'>;

const USER = '5fe9d9ce-5400-4000-8000-000000000001';
const AGENT_ID = '7b1c2a90-0000-4000-8000-000000000002';
const JOB_ID = 'a1b2c3d4-1111-4000-8000-000000000003';
const TX_ID = 'e5f6a7b8-2222-4000-8000-000000000004';
const UUID_RE = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi;

const OBSERVED = `{"state":"OPEN","agentId":"${AGENT_ID}"}`;

const JOBS = [
  makeDeviceJobRow({
    id: JOB_ID,
    kind: 'DRAWER_REQUEST',
    ref: TX_ID,
    status: 'COMMAND_SIMULATED',
    actor: USER,
    created: '2026-11-07T03:00:00.000Z',
  }),
];

const EVENTS = [
  makeDeviceEventRow({
    seq: '3',
    kind: 'JOB_LEASED',
    ref: TX_ID,
    at: '2026-11-07T03:02:00.000Z',
    actor: `agent:${AGENT_ID}`,
    actorName: null,
    detail: `{"job":"${JOB_ID}","leaseId":"L-9","physicalAction":true}`,
  }),
  makeDeviceEventRow({
    seq: '2',
    kind: 'DRAWER_MANUAL_OBSERVATION',
    ref: '',
    at: '2026-11-07T03:01:00.000Z',
    actor: USER,
    actorName: 'Teller 1',
    detail: OBSERVED,
  }),
  makeDeviceEventRow({
    seq: '1',
    kind: 'DRAWER_SKIPPED',
    ref: 'TX-1',
    at: '2026-11-07T03:00:00.000Z',
    actor: USER,
    actorName: 'Teller 1',
    detail: '{}',
  }),
];

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
    device_jobs: JOBS,
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

function text(el: Element | null | undefined): string {
  return (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
}

/** The event rows, newest first. */
function eventRows(): HTMLTableRowElement[] {
  return [...host.querySelectorAll<HTMLTableRowElement>('.events-table tbody tr')];
}

function cell(tr: HTMLTableRowElement, label: string): HTMLElement {
  return tr.querySelector<HTMLElement>(`td[data-label="${label}"]`)!;
}

function detailItems(tr: HTMLTableRowElement): string[] {
  return [...cell(tr, th.colDetail).querySelectorAll('ul.dev-detail > li')].map(text);
}

describe('device log detail on /devices', () => {
  it("DL-1 the verifier's detail reads as labelled lines, not JSON; the list's title keeps the JSON", async () => {
    await mount();
    const row = eventRows()[1]!;
    expect(detailItems(row)).toEqual(['สถานะ: เปิด', 'device agent: 7b1c2a90']);
    const detail = cell(row, th.colDetail);
    expect(text(detail)).not.toContain('{');
    expect(detail.querySelector('ul.dev-detail')?.getAttribute('title')).toBe(OBSERVED);
    expect(detail.querySelector('code')).toBeNull();
  });

  it('DL-2 a shortened id in the detail keeps the full id in its title', async () => {
    await mount();
    const row = eventRows()[1]!;
    const li = cell(row, th.colDetail).querySelectorAll('ul.dev-detail > li')[1]!;
    expect(li.querySelector(`[title="${AGENT_ID}"]`)?.textContent).toBe('7b1c2a90');
  });

  it('DL-3 booleans and other keys read as words; an empty detail is —', async () => {
    await mount();
    const [leased, , skipped] = eventRows();
    expect(detailItems(leased!)).toEqual(['งาน: a1b2c3d4', 'เลขการรับงาน: L-9', 'สั่งอุปกรณ์จริง: ใช่']);
    expect(detailItems(skipped!)).toEqual(['—']);
  });

  it('DL-4 the kinds that had no label now read in Thai', async () => {
    await mount();
    const [leased, , skipped] = eventRows();
    expect(text(cell(leased!, th.colEvent))).toBe('device agent รับงาน');
    expect(text(cell(skipped!, th.colEvent))).toBe('ข้ามการเปิดลิ้นชัก (สาขาไม่มีลิ้นชัก)');
  });

  it('DL-5 an event ref that is a UUID shows 8 characters, the full id in title; a plain ref is unchanged', async () => {
    await mount();
    const [leased, observed, skipped] = eventRows();
    const ref = cell(leased!, th.colRef).querySelector('small')!;
    expect(text(ref)).toBe('e5f6a7b8');
    expect(ref.getAttribute('title')).toBe(TX_ID);
    expect(text(cell(observed!, th.colRef))).toBe('—');
    expect(text(cell(skipped!, th.colRef))).toBe('TX-1');
  });

  it('DL-6 the job table shows no full UUID: id, ref and actor are 8 characters with the full id in title', async () => {
    await mount();
    const table = host.querySelector('.jobs-table')!;
    expect(text(table).match(UUID_RE)).toBeNull();
    const tr = table.querySelector('tbody tr')!;
    const id = tr.querySelector(`td[data-label="${th.colJobTime}"] small`)!;
    expect(text(id)).toBe('a1b2c3d4');
    expect(id.getAttribute('title')).toBe(JOB_ID);
    const ref = tr.querySelector(`td[data-label="${th.colJobKind}"] small`)!;
    expect(text(ref)).toBe('e5f6a7b8');
    expect(ref.getAttribute('title')).toBe(TX_ID);
    const actor = tr.querySelector(`td[data-label="${th.colJobActor}"] span`)!;
    expect(text(actor)).toBe('5fe9d9ce');
    expect(actor.getAttribute('title')).toBe(USER);
  });

  it('DL-7 the event table shows no full UUID outside the actor column (round 12 keeps that column)', async () => {
    await mount();
    for (const tr of eventRows()) {
      for (const label of [th.colEvent, th.colRef, th.colDetail]) {
        expect(text(cell(tr, label)).match(UUID_RE), label).toBeNull();
      }
    }
    expect(text(cell(eventRows()[1]!, th.colActor))).toBe('Teller 1');
  });

  it('DL-8 three events, three rows (no row dropped or added)', async () => {
    await mount();
    expect(eventRows()).toHaveLength(3);
  });
});
