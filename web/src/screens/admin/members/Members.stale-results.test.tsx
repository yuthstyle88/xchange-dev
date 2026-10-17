// #2552: Admin members — defining a new segment must clear the previous segment's snapshotInfo and preview
// (while a failed define keeps them), and taking a snapshot must clear preview before loadPreview so a failed
// preview read never leaves an older preview next to the new snapshot count.
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang } from '../../../shared/i18n';
import { warmModules } from '../../../shared/testing/warm-modules';
import type { AdminMenuProps } from '../menus';
import * as table from './i18n';
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
  const [period] = createSignal({ from: '2026-10-16', to: '2026-10-16' });
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

describe('Admin Members stale snapshot and preview clearing (#2552 §8 & §9)', () => {
  let root: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let defineReplies: Array<{ status: number; body: unknown }>;
  let snapshotReplies: Array<{ status: number; body: unknown }>;
  let previewReplies: Array<{ status: number; body: unknown }>;

  const previewAData = {
    snapshotId: 'snap-A',
    memberCount: 42,
    distribution: {
      avgRecencyDays: 14,
      avgFrequency: 3.5,
      totalValueThb: '150000.00',
    },
    sample: [
      {
        customerId: 'cust-01',
        maskedName: 'Som**** Prasert',
        metrics: {
          recencyDays: 10,
          frequency: 2,
          valueThb: '5000.00',
          tierCode: 'GOLD',
        },
      },
    ],
  };

  beforeEach(() => {
    setLang('th');
    root = document.createElement('div');
    document.body.appendChild(root);
    defineReplies = [];
    snapshotReplies = [];
    previewReplies = [];

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
      const rawUrl =
        typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(rawUrl, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();

      if (method === 'POST' && u.pathname === '/api/segments') {
        const reply = defineReplies.shift() ?? { status: 200, body: { id: 'seg-default', version: 1 } };
        return jsonResponse(reply.status, reply.body);
      }

      if (method === 'POST' && /^\/api\/segments\/[^/]+\/snapshots$/.test(u.pathname)) {
        const reply = snapshotReplies.shift() ?? {
          status: 200,
          body: { id: 'snap-default', memberCount: 42 },
        };
        return jsonResponse(reply.status, reply.body);
      }

      if (method === 'GET' && u.pathname === '/api/v3/views/segment_preview') {
        const reply = previewReplies.shift() ?? {
          status: 200,
          body: { version: 1, changed: true, data: previewAData },
        };
        return jsonResponse(reply.status, reply.body);
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

  it('row 1: Define A, snapshot A, define B -> No snapshot count and no preview shown (while a failed define keeps previous results, §9)', async () => {
    defineReplies = [
      { status: 200, body: { id: 'seg-A', version: 1 } },
      {
        status: 400,
        body: { error: { code: 'validationFailed', message: 'Invalid rule' } },
      },
      { status: 200, body: { id: 'seg-B', version: 1 } },
    ];
    snapshotReplies = [{ status: 200, body: { id: 'snap-A', memberCount: 42 } }];
    previewReplies = [{ status: 200, body: { version: 1, changed: true, data: previewAData } }];

    mountMembers();

    // 1. Define A and snapshot A
    setDefineInputs('Segment A', '30');
    await clickDefine();
    await clickSnapshot();

    expect(root.textContent).toContain(table.th.snapshotTitle);
    expect(root.textContent).toContain('42');
    expect(root.textContent).toContain(table.th.previewTitle);
    expect(root.textContent).toContain('Som**** Prasert');

    // 2. Edge case (§9): A failed define keeps the previous results
    setDefineInputs('Segment B invalid', '15');
    await clickDefine();
    expect(root.querySelector('[role="alert"]')?.textContent).toContain(
      table.th['error.validationFailed'],
    );
    expect(root.textContent).toContain(table.th.snapshotTitle);
    expect(root.textContent).toContain('42');
    expect(root.textContent).toContain(table.th.previewTitle);
    expect(root.textContent).toContain('Som**** Prasert');

    // 3. Successful define B clears both A's snapshot count and A's preview
    setDefineInputs('Segment B', '15');
    await clickDefine();
    expect(root.querySelector('[role="alert"]')).toBeNull();
    expect(root.textContent).not.toContain(table.th.snapshotTitle);
    expect(root.textContent).not.toContain(table.th.previewTitle);
    expect(root.textContent).not.toContain('Som**** Prasert');
  });

  it("row 2: Then snapshot B, preview read fails -> B's count shown, no preview (not A's)", async () => {
    defineReplies = [
      { status: 200, body: { id: 'seg-A', version: 1 } },
      { status: 200, body: { id: 'seg-B', version: 1 } },
    ];
    snapshotReplies = [
      { status: 200, body: { id: 'snap-A', memberCount: 42 } },
      { status: 200, body: { id: 'snap-B1', memberCount: 99 } },
      { status: 200, body: { id: 'snap-B2', memberCount: 88 } },
      { status: 200, body: { id: 'snap-B3', memberCount: 77 } },
    ];
    previewReplies = [
      { status: 200, body: { version: 1, changed: true, data: previewAData } },
      {
        status: 500,
        body: { error: { code: 'serverError', message: 'Preview read failed' } },
      },
      { status: 200, body: { version: 1, changed: true, data: previewAData } },
      {
        status: 500,
        body: { error: { code: 'serverError', message: 'Second preview read failed' } },
      },
    ];

    mountMembers();

    // 1. Define A and snapshot A (loads A's count 42 and preview Som**** Prasert)
    setDefineInputs('Segment A', '30');
    await clickDefine();
    await clickSnapshot();
    expect(root.textContent).toContain('42');
    expect(root.textContent).toContain('Som**** Prasert');

    // 2. Define B, then snapshot B where B's preview read fails -> B's count (99) is shown, no preview
    setDefineInputs('Segment B', '60');
    await clickDefine();
    await clickSnapshot();

    expect(root.textContent).toContain(table.th.snapshotTitle);
    expect(root.textContent).toContain('99');
    expect(root.textContent).not.toContain(table.th.previewTitle);
    expect(root.textContent).not.toContain('Som**** Prasert');

    // 3. Even if a preview was loaded on the current definition and a subsequent snapshot's preview read fails,
    // the new snapshot count (77) is shown and the old preview is cleared before loadPreview
    await clickSnapshot();
    expect(root.textContent).toContain('Som**** Prasert');

    await clickSnapshot();
    expect(root.textContent).toContain(table.th.snapshotTitle);
    expect(root.textContent).toContain('77');
    expect(root.textContent).not.toContain(table.th.previewTitle);
    expect(root.textContent).not.toContain('Som**** Prasert');
  });

  // Kept rows from §8:
  // - "Single define -> snapshot -> preview" is kept and pinned by Members.test.tsx.
  // - "Rule validation, in-flight, keys" is kept and pinned by Members.rule-int.test.tsx, Members.inflight.test.tsx, and Members.keyed.test.tsx.
});
