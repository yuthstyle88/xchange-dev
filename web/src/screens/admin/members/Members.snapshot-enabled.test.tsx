import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang } from '../../../shared/i18n';
import { warmModules } from '../../../shared/testing/warm-modules';
import type { AdminMenuProps } from '../menus';
import { Members } from './Members';

beforeAll(
  () =>
    warmModules(
      () => import('./Members'),
      () => import('./i18n'),
      () => import('../../../shared/api/client'),
      () => import('../../../shared/i18n'),
      () => import('../../../shared/i18n/worded-error'),
    ),
  60_000,
);

function makeProps(): AdminMenuProps {
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-10-14', to: '2026-10-14' });
  const [shown] = createSignal(true);
  return { branch, period, shown };
}

const realSetTimeout = globalThis.setTimeout;
async function settle(): Promise<void> {
  for (let i = 0; i < 10; i++) {
    await new Promise((r) => realSetTimeout(r, 0));
  }
}

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

describe('Admin Members: Snapshot is disabled until a segment is defined (#2634)', () => {
  let root: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let allCalls: { method: string; pathname: string }[];
  let defineCalls: number;
  let snapshotCalls: number;
  let defineResponder: () => Promise<Response>;

  beforeEach(() => {
    setLang('th');
    root = document.createElement('div');
    document.body.appendChild(root);
    allCalls = [];
    defineCalls = 0;
    snapshotCalls = 0;
    defineResponder = async () => jsonResponse(200, { id: 'seg-1', version: 1 });

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
      const rawUrl =
        typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(rawUrl, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      allCalls.push({ method, pathname: u.pathname });

      if (method === 'POST' && u.pathname === '/api/segments') {
        defineCalls++;
        return defineResponder();
      }
      if (method === 'POST' && /^\/api\/segments\/[^/]+\/snapshots$/.test(u.pathname)) {
        snapshotCalls++;
        return jsonResponse(200, { id: 'snap-1', memberCount: 25 });
      }
      if (method === 'GET' && u.pathname === '/api/v3/views/segment_preview') {
        return jsonResponse(200, {
          version: 1,
          changed: true,
          data: {
            snapshotId: 'snap-1',
            memberCount: 25,
            distribution: { avgRecencyDays: 10, avgFrequency: 2, totalValueThb: '10000.00' },
            sample: [],
          },
        });
      }
      return jsonResponse(404, { error: { code: 'notFound', message: 'Not found' } });
    });
  });

  afterEach(async () => {
    await settle();
    dispose?.();
    dispose = undefined;
    root.remove();
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  function mountMembers() {
    dispose = render(() => <Members {...makeProps()} />, root);
  }

  function setName(nameVal: string) {
    const nameInput = root.querySelector('#segment-name') as HTMLInputElement;
    expect(nameInput).not.toBeNull();
    nameInput.value = nameVal;
    nameInput.dispatchEvent(new Event('input', { bubbles: true }));
  }

  function defineButton(): HTMLButtonElement {
    const btn = root.querySelector('button[data-action="define"]') as HTMLButtonElement;
    expect(btn).not.toBeNull();
    return btn;
  }

  function snapshotButton(): HTMLButtonElement {
    const btn = root.querySelector('button[data-action="snapshot"]') as HTMLButtonElement;
    expect(btn).not.toBeNull();
    return btn;
  }

  it('row 1: Screen opened, nothing defined -> Snapshot disabled; no request', async () => {
    mountMembers();
    await settle();

    const snapBtn = snapshotButton();
    expect(snapBtn.disabled).toBe(true);

    snapBtn.click();
    await settle();

    expect(snapshotCalls).toBe(0);
    expect(allCalls).toHaveLength(0);
  });

  it('row 2: Define succeeds -> Snapshot enabled', async () => {
    mountMembers();
    setName('VIP Gold');

    const snapBtn = snapshotButton();
    expect(snapBtn.disabled).toBe(true);

    defineButton().click();
    await settle();

    expect(defineCalls).toBe(1);
    expect(root.querySelector('[role="alert"]')).toBeNull();
    expect(snapBtn.disabled).toBe(false);

    snapBtn.click();
    await settle();
    expect(snapshotCalls).toBe(1);
  });

  it('row 3: Define fails with no earlier definition -> Snapshot stays disabled', async () => {
    defineResponder = async () =>
      jsonResponse(422, { error: { code: 'validationFailed', message: 'Invalid rule' } });

    mountMembers();
    setName('VIP Gold');

    const snapBtn = snapshotButton();
    expect(snapBtn.disabled).toBe(true);

    defineButton().click();
    await settle();

    expect(defineCalls).toBe(1);
    expect(root.querySelector('[role="alert"]')).not.toBeNull();
    expect(snapBtn.disabled).toBe(true);

    snapBtn.click();
    await settle();
    expect(snapshotCalls).toBe(0);
  });
});
