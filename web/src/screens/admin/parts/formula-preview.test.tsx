// Pricing formula FE T1 up-front tests (docs/design/blueprint/pricing-formula-web.md §3 P1–P8), written before the
// implementation by the sub-controller claude/studio-claude-3. Do not edit, delete or weaken; add tests in a file of
// your own. Contract: `pricingFormulaPreview` POST /api/pricing/formula/preview
// (api/src/presentation/http/v3/commands/pricing_formula_preview.rs:101-110).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../shared/api/client';
import { setLang } from '../../../shared/i18n';
import {
  makeFormulaCheckRowDto,
  makePricingFormulaPreviewOutput,
} from '../../../shared/testing/fixtures';
import {
  createFormulaPreview,
  PREVIEW_DELAY_MS,
  ROUTE_PRICING_FORMULA_PREVIEW,
  type FormulaPreview,
} from './formula-preview';

interface Sent {
  url: string;
  body: Record<string, unknown>;
  resolve: (r: Response) => void;
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

function answer(currency: string): Response {
  return json(
    makePricingFormulaPreviewOutput({
      ok: true,
      rows: [makeFormulaCheckRowDto({ branch: 'MAIN', currency, mode: 'AUTO', buy: '33.7', sell: '33.9' })],
    }),
  );
}

describe('pricing-formula-web T1: createFormulaPreview', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let sent: Sent[];
  let preview: FormulaPreview;
  const [buy, setBuy] = createSignal('');
  const [sell, setSell] = createSignal('');
  const [branch, setBranch] = createSignal('MAIN');
  const [enabled, setEnabled] = createSignal(true);

  beforeEach(() => {
    vi.useFakeTimers();
    setLang('th');
    setBuy('ref_buy');
    setSell('ref_sell');
    setBranch('MAIN');
    setEnabled(true);
    sent = [];
    host = document.createElement('div');
    document.body.appendChild(host);
    vi.stubGlobal(
      'fetch',
      vi.fn(
        (input: RequestInfo | URL, init?: RequestInit) =>
          new Promise<Response>((resolve) => {
            const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
            sent.push({ url, body: JSON.parse(String(init?.body ?? '{}')), resolve });
          }),
      ),
    );
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  function mount(): void {
    function Probe(): JSX.Element {
      preview = createFormulaPreview({
        api: new ApiClient(),
        branch,
        scope: 'ORGANISATION',
        buy,
        sell,
        enabled,
      });
      return null;
    }
    dispose = render(() => <Probe />, host);
  }

  async function settle(ms = PREVIEW_DELAY_MS): Promise<void> {
    await vi.advanceTimersByTimeAsync(ms);
  }

  it('P1: one call once the texts stop changing, with the texts trimmed; the answer becomes result()', async () => {
    setBuy('  ref_mid - 0.05 ');
    setSell('ref_mid + 0.05\n');
    mount();
    await settle(PREVIEW_DELAY_MS - 1);
    expect(sent).toHaveLength(0);
    await settle(1);
    expect(sent).toHaveLength(1);
    expect(sent[0].url).toContain(ROUTE_PRICING_FORMULA_PREVIEW);
    expect(sent[0].body).toEqual({
      branch: 'MAIN',
      buyFormula: 'ref_mid - 0.05',
      sellFormula: 'ref_mid + 0.05',
      scope: 'ORGANISATION',
    });
    sent[0].resolve(answer('USD'));
    await vi.waitFor(() => expect(preview.result()?.rows[0]?.currency).toBe('USD'));
  });

  it('P2: edits within the delay send one call with the last texts', async () => {
    mount();
    await settle(100);
    setBuy('ref_buy * 0.99');
    await settle(100);
    setBuy('ref_buy * 0.98');
    await settle(100);
    setSell('ref_sell * 1.02');
    await settle(PREVIEW_DELAY_MS);
    expect(sent).toHaveLength(1);
    expect(sent[0].body).toMatchObject({ buyFormula: 'ref_buy * 0.98', sellFormula: 'ref_sell * 1.02' });
  });

  it('P3: a late answer for older texts is dropped', async () => {
    mount();
    await settle();
    expect(sent).toHaveLength(1);
    setBuy('ref_buy * 0.99');
    await settle();
    expect(sent).toHaveLength(2);
    sent[1].resolve(answer('EUR'));
    await vi.waitFor(() => expect(preview.result()?.rows[0]?.currency).toBe('EUR'));
    sent[0].resolve(answer('USD'));
    await settle(10);
    expect(preview.result()?.rows[0]?.currency).toBe('EUR');
  });

  it('P4: a blank text sends nothing and clears the result', async () => {
    setSell('   ');
    mount();
    await settle(PREVIEW_DELAY_MS * 3);
    expect(sent).toHaveLength(0);
    expect(preview.result()).toBeNull();
  });

  it('P5: disabled sends nothing', async () => {
    setEnabled(false);
    mount();
    setBuy('ref_buy * 0.99');
    await settle(PREVIEW_DELAY_MS * 3);
    expect(sent).toHaveLength(0);
  });

  it('P6: a 403 stops the preview until the branch changes', async () => {
    mount();
    await settle();
    sent[0].resolve(json({ error: { code: 'roleNotAllowedHere', message: 'EXECUTIVE only' } }, 403));
    await vi.waitFor(() => expect(preview.stopped()).toBe(true));
    expect(preview.error()).toBeTruthy();
    setBuy('ref_buy * 0.99');
    await settle(PREVIEW_DELAY_MS * 3);
    expect(sent).toHaveLength(1);
    setBranch('B2');
    await settle();
    expect(preview.stopped()).toBe(false);
    expect(sent).toHaveLength(2);
    expect(sent[1].body).toMatchObject({ branch: 'B2' });
  });

  it('P7: a server failure is worded, clears the result, and the next edit calls again', async () => {
    mount();
    await settle();
    sent[0].resolve(json({ error: { code: 'serverError', message: 'boom' } }, 500));
    await vi.waitFor(() => expect(preview.error()).toBeTruthy());
    expect(preview.result()).toBeNull();
    expect(preview.stopped()).toBe(false);
    setBuy('ref_buy * 0.99');
    await settle();
    expect(sent).toHaveLength(2);
  });

  it('P8: pending while a call is in flight', async () => {
    mount();
    await settle();
    expect(preview.pending()).toBe(true);
    sent[0].resolve(answer('USD'));
    await vi.waitFor(() => expect(preview.pending()).toBe(false));
  });
});
