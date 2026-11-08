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

const UUID_ACTOR = '9a1b2c3d-1111-4222-8333-444455556666';
const NON_UUID_ACTOR = 'owner';
const NAMED_ACTOR = 'user:11112222-3333-4444-5555-666677778888';

function eventRow(seq: string, actor: string, actorName: string | null) {
  return makeDeviceEventRow({ seq, kind: 'PRINT_RECEIPT', ref: `TX-${seq}`, actor, actorName, detail: '{}' });
}

const EVENTS = [
  eventRow('3', UUID_ACTOR, null),
  eventRow('2', NON_UUID_ACTOR, null),
  eventRow('1', NAMED_ACTOR, 'Teller 1'),
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

function actorCells(): HTMLElement[] {
  return [...host.querySelectorAll<HTMLElement>(`.events-table tbody td[data-label="${th.colActor}"]`)];
}

describe('DevicesScreen actor fallback (OL-8)', () => {
  it('actorName: null, actor a UUID prints 8-char shortId and title has full UUID', async () => {
    await mount();
    const [uuidCell] = actorCells();
    expect(uuidCell!.textContent?.trim()).toBe('9a1b2c3d');
    expect(uuidCell!.querySelector('.dev-id')?.getAttribute('title')).toBe(UUID_ACTOR);
  });

  it('actorName: null, actor not a UUID prints unchanged cell text', async () => {
    await mount();
    const [, nonUuidCell] = actorCells();
    expect(nonUuidCell!.textContent?.trim()).toBe('owner');
    expect(nonUuidCell!.querySelector('.dev-id')?.getAttribute('title')).toBe('owner');
  });

  it('actorName: "Teller 1" renders .dev-actor-name "Teller 1" unchanged', async () => {
    await mount();
    const [, , namedCell] = actorCells();
    expect(namedCell!.querySelector('.dev-actor-name')?.textContent?.trim()).toBe('Teller 1');
    expect(namedCell!.querySelector('.dev-actor-name')?.getAttribute('title')).toBe(NAMED_ACTOR);
  });
});
