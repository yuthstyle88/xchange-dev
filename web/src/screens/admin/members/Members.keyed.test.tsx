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

interface CapturedWrite {
  method: string;
  pathname: string;
  idempotencyKey: string;
  rawBody: string;
}

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

describe('KW-5 (#2079): Members screen writes idempotency-key lifecycle', () => {
  let root: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let defineCalls: CapturedWrite[];
  let snapshotCalls: CapturedWrite[];
  let defineResponder: (callIndex: number, req: CapturedWrite) => Promise<Response>;
  let snapshotResponder: (callIndex: number, req: CapturedWrite) => Promise<Response>;

  beforeEach(() => {
    setLang('th');
    root = document.createElement('div');
    document.body.appendChild(root);
    defineCalls = [];
    snapshotCalls = [];

    defineResponder = async (idx) =>
      jsonResponse(200, { id: `seg-${idx + 1}`, version: 1 });
    snapshotResponder = async (idx) =>
      jsonResponse(200, { id: `snap-${idx + 1}`, memberCount: 25 });

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
      const rawUrl =
        typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(rawUrl, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      const headers = new Headers(init?.headers);
      const idempotencyKey = headers.get('idempotency-key') ?? '';
      const rawBody = typeof init?.body === 'string' ? init.body : '';

      if (method === 'POST' && u.pathname === '/api/segments') {
        const entry: CapturedWrite = {
          method,
          pathname: u.pathname,
          idempotencyKey,
          rawBody,
        };
        const idx = defineCalls.length;
        defineCalls.push(entry);
        return defineResponder(idx, entry);
      }

      if (method === 'POST' && /^\/api\/segments\/[^/]+\/snapshots$/.test(u.pathname)) {
        const entry: CapturedWrite = {
          method,
          pathname: u.pathname,
          idempotencyKey,
          rawBody,
        };
        const idx = snapshotCalls.length;
        snapshotCalls.push(entry);
        return snapshotResponder(idx, entry);
      }

      if (method === 'GET' && u.pathname === '/api/v3/views/segment_preview') {
        return jsonResponse(200, {
          version: 1,
          changed: true,
          data: {
            snapshotId: u.searchParams.get('id') ?? 'snap-1',
            memberCount: 25,
            distribution: {
              avgRecencyDays: 10,
              avgFrequency: 2,
              totalValueThb: '10000.00',
            },
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

  function setDefineInputs(nameVal: string, recencyVal = '30') {
    const nameInput = root.querySelector('#segment-name') as HTMLInputElement;
    expect(nameInput).not.toBeNull();
    nameInput.value = nameVal;
    nameInput.dispatchEvent(new Event('input', { bubbles: true }));

    const recencyInput = root.querySelector('#segment-recency-days') as HTMLInputElement;
    expect(recencyInput).not.toBeNull();
    recencyInput.value = recencyVal;
    recencyInput.dispatchEvent(new Event('input', { bubbles: true }));
  }

  async function clickDefine() {
    const btn = root.querySelector('button[data-action="define"]') as HTMLButtonElement;
    expect(btn).not.toBeNull();
    btn.click();
    await settle();
  }

  async function clickSnapshot() {
    const btn = root.querySelector('button[data-action="snapshot"]') as HTMLButtonElement;
    expect(btn).not.toBeNull();
    btn.click();
    await settle();
  }

  describe('segment_define (POST /api/segments)', () => {
    it('row 1: submit -> network TypeError -> submit again with the same input sends 2 requests with the same key and byte-equal body', async () => {
      defineResponder = async (idx) => {
        if (idx === 0) throw new TypeError('Failed to fetch');
        return jsonResponse(200, { id: 'seg-1', version: 1 });
      };

      mountMembers();
      setDefineInputs('VIP Gold', '30');

      await clickDefine();
      await clickDefine();

      expect(defineCalls).toHaveLength(2);
      expect(defineCalls[0]!.idempotencyKey).toBeTruthy();
      expect(defineCalls[1]!.idempotencyKey).toBe(defineCalls[0]!.idempotencyKey);
      expect(defineCalls[1]!.rawBody).toBe(defineCalls[0]!.rawBody);
    });

    it('row 2: submit -> 500 -> submit again reuses the same key', async () => {
      defineResponder = async (idx) => {
        if (idx === 0) {
          return jsonResponse(500, {
            error: { code: 'serverError', message: 'Internal error' },
          });
        }
        return jsonResponse(200, { id: 'seg-1', version: 1 });
      };

      mountMembers();
      setDefineInputs('VIP Gold', '30');

      await clickDefine();
      await clickDefine();

      expect(defineCalls).toHaveLength(2);
      expect(defineCalls[0]!.idempotencyKey).toBeTruthy();
      expect(defineCalls[1]!.idempotencyKey).toBe(defineCalls[0]!.idempotencyKey);
      expect(defineCalls[1]!.rawBody).toBe(defineCalls[0]!.rawBody);
    });

    it('row 3: submit -> 4xx refusal -> submit again mints a new key', async () => {
      defineResponder = async (idx) => {
        if (idx === 0) {
          return jsonResponse(400, {
            error: { code: 'validationFailed', field: 'name', message: 'Invalid name' },
          });
        }
        return jsonResponse(200, { id: 'seg-1', version: 1 });
      };

      mountMembers();
      setDefineInputs('VIP Gold', '30');

      await clickDefine();
      await clickDefine();

      expect(defineCalls).toHaveLength(2);
      expect(defineCalls[0]!.idempotencyKey).toBeTruthy();
      expect(defineCalls[1]!.idempotencyKey).toBeTruthy();
      expect(defineCalls[1]!.idempotencyKey).not.toBe(defineCalls[0]!.idempotencyKey);
      expect(defineCalls[1]!.rawBody).toBe(defineCalls[0]!.rawBody);
    });

    it('row 4: submit -> network error -> edit the input -> submit mints a new key', async () => {
      defineResponder = async (idx) => {
        if (idx === 0) throw new TypeError('Failed to fetch');
        return jsonResponse(200, { id: 'seg-1', version: 1 });
      };

      mountMembers();
      setDefineInputs('VIP Gold', '30');
      await clickDefine();

      setDefineInputs('VIP Platinum', '30');
      await clickDefine();

      expect(defineCalls).toHaveLength(2);
      expect(defineCalls[0]!.idempotencyKey).toBeTruthy();
      expect(defineCalls[1]!.idempotencyKey).toBeTruthy();
      expect(defineCalls[1]!.idempotencyKey).not.toBe(defineCalls[0]!.idempotencyKey);
      expect(defineCalls[1]!.rawBody).not.toBe(defineCalls[0]!.rawBody);
    });

    it('row 5: submit -> 2xx -> submit the same input again mints a new key', async () => {
      mountMembers();
      setDefineInputs('VIP Gold', '30');

      await clickDefine();
      await clickDefine();

      expect(defineCalls).toHaveLength(2);
      expect(defineCalls[0]!.idempotencyKey).toBeTruthy();
      expect(defineCalls[1]!.idempotencyKey).toBeTruthy();
      expect(defineCalls[1]!.idempotencyKey).not.toBe(defineCalls[0]!.idempotencyKey);
      expect(defineCalls[1]!.rawBody).toBe(defineCalls[0]!.rawBody);
    });
  });

  describe('segment_snapshot (POST /api/segments/{id}/snapshots)', () => {
    it('row 1: submit -> network TypeError -> submit again with the same input sends 2 requests with the same key and byte-equal body', async () => {
      snapshotResponder = async (idx) => {
        if (idx === 0) throw new TypeError('Failed to fetch');
        return jsonResponse(200, { id: 'snap-1', memberCount: 25 });
      };

      mountMembers();
      setDefineInputs('VIP Gold', '30');
      await clickDefine();

      await clickSnapshot();
      await clickSnapshot();

      expect(snapshotCalls).toHaveLength(2);
      expect(snapshotCalls[0]!.pathname).toBe('/api/segments/seg-1/snapshots');
      expect(snapshotCalls[0]!.idempotencyKey).toBeTruthy();
      expect(snapshotCalls[1]!.idempotencyKey).toBe(snapshotCalls[0]!.idempotencyKey);
      expect(snapshotCalls[1]!.rawBody).toBe(snapshotCalls[0]!.rawBody);
    });

    it('row 2: submit -> 500 -> submit again reuses the same key', async () => {
      snapshotResponder = async (idx) => {
        if (idx === 0) {
          return jsonResponse(500, {
            error: { code: 'serverError', message: 'Internal error' },
          });
        }
        return jsonResponse(200, { id: 'snap-1', memberCount: 25 });
      };

      mountMembers();
      setDefineInputs('VIP Gold', '30');
      await clickDefine();

      await clickSnapshot();
      await clickSnapshot();

      expect(snapshotCalls).toHaveLength(2);
      expect(snapshotCalls[0]!.idempotencyKey).toBeTruthy();
      expect(snapshotCalls[1]!.idempotencyKey).toBe(snapshotCalls[0]!.idempotencyKey);
      expect(snapshotCalls[1]!.rawBody).toBe(snapshotCalls[0]!.rawBody);
    });

    it('row 3: submit -> 4xx refusal -> submit again mints a new key', async () => {
      snapshotResponder = async (idx) => {
        if (idx === 0) {
          return jsonResponse(400, {
            error: { code: 'outOfScope', message: 'Out of scope' },
          });
        }
        return jsonResponse(200, { id: 'snap-1', memberCount: 25 });
      };

      mountMembers();
      setDefineInputs('VIP Gold', '30');
      await clickDefine();

      await clickSnapshot();
      await clickSnapshot();

      expect(snapshotCalls).toHaveLength(2);
      expect(snapshotCalls[0]!.idempotencyKey).toBeTruthy();
      expect(snapshotCalls[1]!.idempotencyKey).toBeTruthy();
      expect(snapshotCalls[1]!.idempotencyKey).not.toBe(snapshotCalls[0]!.idempotencyKey);
      expect(snapshotCalls[1]!.rawBody).toBe(snapshotCalls[0]!.rawBody);
    });

    it('row 4: submit -> network error -> edit the input (new segment definition id) -> submit mints a new key', async () => {
      snapshotResponder = async (idx) => {
        if (idx === 0) throw new TypeError('Failed to fetch');
        return jsonResponse(200, { id: 'snap-2', memberCount: 30 });
      };

      mountMembers();
      setDefineInputs('VIP Gold', '30');
      await clickDefine(); // produces seg-1
      await clickSnapshot(); // fails with TypeError for { id: 'seg-1' }

      setDefineInputs('VIP Platinum', '60');
      await clickDefine(); // produces seg-2
      await clickSnapshot(); // submits { id: 'seg-2' }

      expect(snapshotCalls).toHaveLength(2);
      expect(snapshotCalls[0]!.pathname).toBe('/api/segments/seg-1/snapshots');
      expect(snapshotCalls[1]!.pathname).toBe('/api/segments/seg-2/snapshots');
      expect(snapshotCalls[0]!.idempotencyKey).toBeTruthy();
      expect(snapshotCalls[1]!.idempotencyKey).toBeTruthy();
      expect(snapshotCalls[1]!.idempotencyKey).not.toBe(snapshotCalls[0]!.idempotencyKey);
      expect(snapshotCalls[1]!.rawBody).not.toBe(snapshotCalls[0]!.rawBody);
    });

    it('row 5: submit -> 2xx -> submit the same input again mints a new key', async () => {
      mountMembers();
      setDefineInputs('VIP Gold', '30');
      await clickDefine();

      await clickSnapshot();
      await clickSnapshot();

      expect(snapshotCalls).toHaveLength(2);
      expect(snapshotCalls[0]!.idempotencyKey).toBeTruthy();
      expect(snapshotCalls[1]!.idempotencyKey).toBeTruthy();
      expect(snapshotCalls[1]!.idempotencyKey).not.toBe(snapshotCalls[0]!.idempotencyKey);
      expect(snapshotCalls[1]!.rawBody).toBe(snapshotCalls[0]!.rawBody);
    });
  });
});
