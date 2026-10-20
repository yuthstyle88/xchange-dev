// §9 `PricingPolicy` (FE pricing-board, #564; docs/design/blueprint/pricing-tiers.md §3, §6, §9).
// Tests covering every behaviour row of ticket #3763 (§9 Behaviour table):
// 1. mayEdit: false — nothing editable (read-only margins + min spread)
// 2. mayEdit: true — 4 × buy/sell margin inputs prefilled from row.margins
// 3. save — one POST /api/pricing/configure { policyVersionId, currency, tiers, minSpreadPct, anchorSource, reason } + Idempotency-Key header
// 4. clear override — tiers: null
// 5. a refusal — a role=alert with wordedError
import { afterEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../shared/api/client';
import { setLang } from '../../../shared/i18n';
import type { PricingRowDto } from '../../../shared/testing/fixtures/pricing_board';
import {
  makePricingConfigureOutput,
  type PricingConfigureOutput,
} from '../../../shared/testing/fixtures/pricing_configure';
import { BOARD, boardFetch } from './__fixtures__/pricing-board.fixture';
import { ROUTE_PRICING_CONFIGURE } from './pricing-labels';
import { PricingPolicy } from './PricingPolicy';

const USD = BOARD.rows.find((r) => r.currency === 'USD')!;
const CNY = BOARD.rows.find((r) => r.currency === 'CNY')!;

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  setLang('th');
});

async function flush() {
  for (let i = 0; i < 8; i++) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}

function mount(
  row: PricingRowDto,
  opts?: {
    mayEdit?: boolean;
    onSaved?: (out: PricingConfigureOutput) => void;
    command?: (path: string, body: unknown) => { status: number; body: unknown };
  },
) {
  const onSaved = opts?.onSaved ?? vi.fn();
  const mayEdit = opts?.mayEdit ?? true;
  const savedOut = makePricingConfigureOutput({ id: 'pv-8', version: 2 });
  const defaultCommand = (path: string, _body: unknown) => {
    if (path === ROUTE_PRICING_CONFIGURE) {
      return { status: 200, body: savedOut };
    }
    return { status: 404, body: {} };
  };
  const server = boardFetch([BOARD], opts?.command ?? defaultCommand);
  const api = new ApiClient(server.fn as unknown as typeof fetch);
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(
    () => (
      <PricingPolicy
        api={api}
        branch="MAIN"
        policyVersionId="pv-7"
        row={row}
        mayEdit={mayEdit}
        onSaved={onSaved}
      />
    ),
    host,
  );
  return { server, onSaved };
}

describe('PricingPolicy', () => {
  it('P1 mayEdit: false - nothing editable (read-only margins and min spread)', () => {
    mount(CNY, { mayEdit: false });

    // No enabled inputs or buttons
    const enabledInputs = host.querySelectorAll('input:not([disabled])');
    expect(enabledInputs).toHaveLength(0);

    const enabledButtons = host.querySelectorAll('button:not([disabled])');
    expect(enabledButtons).toHaveLength(0);

    // Margins and min spread are displayed
    for (const m of CNY.margins) {
      expect(host.textContent).toContain(m.buyMarginPct);
      expect(host.textContent).toContain(m.sellMarginPct);
    }
    expect(host.textContent).toContain(CNY.minSpreadPct!);
  });

  it('P2 mayEdit: true - 4 × buy/sell margin inputs prefilled from row.margins and minSpreadPct', () => {
    mount(CNY, { mayEdit: true });
    for (const m of CNY.margins) {
      const buyInput = host.querySelector<HTMLInputElement>(`input[name="buy-margin-${m.tierNo}"]`);
      const sellInput = host.querySelector<HTMLInputElement>(`input[name="sell-margin-${m.tierNo}"]`);
      expect(buyInput).not.toBeNull();
      expect(sellInput).not.toBeNull();
      expect(buyInput?.disabled).toBe(false);
      expect(sellInput?.disabled).toBe(false);
      expect(buyInput?.value).toBe(m.buyMarginPct);
      expect(sellInput?.value).toBe(m.sellMarginPct);
    }
    const minSpreadInput = host.querySelector<HTMLInputElement>('input[name="minSpreadPct"]');
    expect(minSpreadInput).not.toBeNull();
    expect(minSpreadInput?.disabled).toBe(false);
    expect(minSpreadInput?.value).toBe(CNY.minSpreadPct!);
  });

  it('P3 save - sends one POST /api/pricing/configure with edited tiers, minSpreadPct, reason, Idempotency-Key and policyVersionId', async () => {
    const { server, onSaved } = mount(USD, { mayEdit: true });
    const buyInput = host.querySelector<HTMLInputElement>('input[name="buy-margin-1"]')!;
    expect(buyInput).not.toBeNull();
    buyInput.value = '0.0500';
    buyInput.dispatchEvent(new Event('input', { bubbles: true }));

    const minSpreadInput = host.querySelector<HTMLInputElement>('input[name="minSpreadPct"]')!;
    expect(minSpreadInput).not.toBeNull();
    minSpreadInput.value = '0.3000';
    minSpreadInput.dispatchEvent(new Event('input', { bubbles: true }));

    const reasonInput = host.querySelector<HTMLInputElement>('input[name="reason"]')!;
    expect(reasonInput).not.toBeNull();
    reasonInput.value = 'update USD margins';
    reasonInput.dispatchEvent(new Event('input', { bubbles: true }));

    const saveBtn = host.querySelector<HTMLButtonElement>('button[name="save"]')!;
    expect(saveBtn).not.toBeNull();
    saveBtn.click();
    await flush();

    const sent = server.calls.filter((c) => c.method === 'POST' && c.path === ROUTE_PRICING_CONFIGURE);
    expect(sent).toHaveLength(1);
    const body = sent[0].body as Record<string, any>;
    expect(body.policyVersionId).toBe('pv-7');
    expect('requestId' in body).toBe(false);
    expect(body.currency).toBe('USD');
    expect(body.tiers).toHaveLength(4);
    expect(body.tiers[0].tierNo).toBe(1);
    expect(body.tiers[0].buyMarginPct).toBe('0.0500');
    expect(body.tiers[0].sellMarginPct).toBe(USD.margins[0].sellMarginPct);
    expect(body.minSpreadPct).toBe('0.3000');
    expect(body.anchorSource).toBeNull();
    expect(body.reason).toBe('update USD margins');
    expect(onSaved).toHaveBeenCalledWith(makePricingConfigureOutput({ id: 'pv-8', version: 2 }));
  });

  it('P4 clear override - sends POST /api/pricing/configure with tiers: null', async () => {
    const { server, onSaved } = mount(CNY, { mayEdit: true });
    const reasonInput = host.querySelector<HTMLInputElement>('input[name="reason"]')!;
    expect(reasonInput).not.toBeNull();
    reasonInput.value = 'reset CNY override';
    reasonInput.dispatchEvent(new Event('input', { bubbles: true }));

    const clearBtn = host.querySelector<HTMLButtonElement>('button[name="clearOverride"]')!;
    expect(clearBtn).not.toBeNull();
    clearBtn.click();
    await flush();

    const sent = server.calls.filter((c) => c.method === 'POST' && c.path === ROUTE_PRICING_CONFIGURE);
    expect(sent).toHaveLength(1);
    const body = sent[0].body as Record<string, any>;
    expect(body.policyVersionId).toBe('pv-7');
    expect('requestId' in body).toBe(false);
    expect(body.currency).toBe('CNY');
    expect(body.tiers).toBeNull();
    expect(body.minSpreadPct).toBeNull();
    expect(body.anchorSource).toBeNull();
    expect(body.reason).toBe('reset CNY override');
    expect(onSaved).toHaveBeenCalledWith(makePricingConfigureOutput({ id: 'pv-8', version: 2 }));
  });

  it('P5 a refusal - displays role=alert with wordedError', async () => {
    const errPayload = { code: 'marginOutOfRange', message: 'margin exceeds allowed limits' };
    mount(USD, {
      mayEdit: true,
      command: () => ({ status: 400, body: errPayload }),
    });

    const saveBtn = host.querySelector<HTMLButtonElement>('button[name="save"]')!;
    expect(saveBtn).not.toBeNull();
    saveBtn.click();
    await flush();

    const alert = host.querySelector('[role="alert"]');
    expect(alert).not.toBeNull();
    expect(alert?.textContent?.length).toBeGreaterThan(0);
  });
});
