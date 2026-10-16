// KW-3 (#2021): idempotency-key lifecycle tests for Campaigns (design `docs/design/xchange-v3-web.md` §12.24).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { Campaigns } from './Campaigns';
import * as table from './i18n';

const LIST_VIEW = '/api/v3/views/campaigns';
const DETAIL_VIEW = '/api/v3/views/campaign';
const SAVE_DRAFT = '/api/campaigns/cmp-1/draft';

interface SentWrite {
  method: string;
  path: string;
  key: string | null;
  body: string;
}

type StepReply = { status: number; body: unknown } | 'network';

describe('KW-3: Campaigns idempotency key lifecycle (#2021 §9)', () => {
  let root: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let writes: SentWrite[];
  let queue: StepReply[];

  const sampleList = {
    campaigns: [
      {
        id: 'cmp-1',
        name: 'Golden October',
        versionId: 'ver-1',
        version: 1,
        status: 'DRAFT',
        startsAt: '2026-10-01T00:00:00+07:00',
        endsAt: '2026-10-31T23:59:59+07:00',
        budgetThb: '100000.00',
        budgetUsedThb: '0.00',
        budgetLeftThb: '100000.00',
        uses: 0,
        maxUses: 100,
        period: {
          redemptions: 0,
          priceCostThb: '0.00',
          rewardExpenseThb: '0.00',
        },
      },
    ],
  };

  const sampleDetail = {
    campaign: {
      id: 'cmp-1',
      name: 'Golden October',
      currentVersionId: 'ver-1',
      versions: [
        {
          id: 'ver-1',
          version: 1,
          status: 'DRAFT',
          startsAt: '2026-10-01T00:00:00+07:00',
          endsAt: '2026-10-31T23:59:59+07:00',
          budgetThb: '100000.00',
          maxUses: 100,
          stackGroup: 'DEFAULT',
          priority: 10,
          branches: ['MAIN'],
          rules: [],
          simulations: [],
          approval: null,
        },
      ],
    },
  };

  beforeEach(() => {
    setLang('th');
    writes = [];
    queue = [];
    root = document.createElement('div');
    document.body.appendChild(root);

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
      const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(raw, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      const headers = new Headers(init?.headers);

      if (method === 'GET' && u.pathname === LIST_VIEW) {
        return new Response(
          JSON.stringify({ version: 1, changed: true, data: structuredClone(sampleList) }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }

      if (method === 'GET' && u.pathname === DETAIL_VIEW) {
        return new Response(
          JSON.stringify({ version: 1, changed: true, data: structuredClone(sampleDetail) }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }

      if (method === 'PUT' && u.pathname === SAVE_DRAFT) {
        const body = typeof init?.body === 'string' ? init.body : '';
        writes.push({ method, path: u.pathname, key: headers.get('idempotency-key'), body });
        const step = queue.shift() ?? { status: 200, body: { versionId: 'ver-1', version: 1 } };
        if (step === 'network') {
          throw new TypeError('Failed to fetch');
        }
        return new Response(JSON.stringify(step.body), {
          status: step.status,
          headers: { 'content-type': 'application/json' },
        });
      }

      return new Response(JSON.stringify({ version: 1, changed: false }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });
    });
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    root.remove();
    vi.restoreAllMocks();
  });

  async function settle(): Promise<void> {
    for (let i = 0; i < 8; i++) {
      await new Promise((r) => setTimeout(r, 0));
    }
  }

  async function mountAndOpenCampaign(): Promise<void> {
    const [branch] = createSignal('MAIN');
    const [period] = createSignal({ from: '2026-10-01', to: '2026-10-31' });
    const [shown] = createSignal(true);
    dispose = render(() => <Campaigns branch={branch} period={period} shown={shown} />, root);
    await settle();

    const row = root.querySelector('[data-campaign-id="cmp-1"]') as HTMLElement | null;
    expect(row, 'campaign row exists').not.toBeNull();
    row!.click();
    await settle();
  }

  function saveButton(): HTMLButtonElement {
    const btn = Array.from(root.querySelectorAll('button')).find(
      (b) => b.textContent?.trim() === table.th.save,
    );
    expect(btn, 'save button exists').toBeDefined();
    return btn!;
  }

  function budgetInput(): HTMLInputElement {
    const input = root.querySelector('input[data-field="budgetThb"]') as HTMLInputElement | null;
    expect(input, 'budgetThb input exists').not.toBeNull();
    return input!;
  }

  async function clickSave(): Promise<void> {
    saveButton().click();
    await settle();
  }

  function setInputValue(input: HTMLInputElement, value: string): void {
    input.value = value;
    input.dispatchEvent(new Event('input', { bubbles: true }));
    input.dispatchEvent(new Event('change', { bubbles: true }));
  }

  it('row 1: submit -> network TypeError -> submit again with the same input sends 2 requests with the same key and byte-equal body', async () => {
    await mountAndOpenCampaign();
    queue.push('network', { status: 200, body: { versionId: 'ver-1', version: 1 } });

    await clickSave();
    await clickSave();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBe(writes[0]!.key);
    expect(writes[1]!.body).toBe(writes[0]!.body);
  });

  it('row 2: submit -> 500 -> submit again sends 2 requests with the same key', async () => {
    await mountAndOpenCampaign();
    queue.push(
      { status: 500, body: { error: { code: 'validationFailed', message: 'transient 500' } } },
      { status: 200, body: { versionId: 'ver-1', version: 1 } },
    );

    await clickSave();
    await clickSave();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBe(writes[0]!.key);
    expect(writes[1]!.body).toBe(writes[0]!.body);
  });

  it('row 3: submit -> 4xx refusal -> submit again sends a new key', async () => {
    await mountAndOpenCampaign();
    queue.push(
      { status: 400, body: { error: { code: 'validationFailed', message: 'invalid budget' } } },
      { status: 200, body: { versionId: 'ver-1', version: 1 } },
    );

    await clickSave();
    await clickSave();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
  });

  it('row 4: submit -> network error -> edit the input -> submit sends a new key', async () => {
    await mountAndOpenCampaign();
    queue.push('network', { status: 200, body: { versionId: 'ver-1', version: 1 } });

    await clickSave();
    setInputValue(budgetInput(), '120000.00');
    await clickSave();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
    expect(writes[1]!.body).not.toBe(writes[0]!.body);
  });

  it('row 5: submit -> 2xx -> submit the same input again sends a new key', async () => {
    await mountAndOpenCampaign();
    queue.push(
      { status: 200, body: { versionId: 'ver-1', version: 1 } },
      { status: 200, body: { versionId: 'ver-1', version: 1 } },
    );

    await clickSave();
    await clickSave();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.body).toBe(writes[0]!.body);
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
  });
});
