// V.9 `DisplaySettings` (PV-j, pricing-tiers.md §15.8): the rate-board display settings of a branch.
// Up-front tests for ticket #5201, one `it` per behaviour-table row (S1..S12):
// tiers checkboxes, the bounded featured list (add from a select, remove per code, at most
// DISPLAY_FEATURED_MAX), highlight / allMarket, one keyed POST /api/pricing/display carrying all four fields,
// `{clear: true}` only for a branch's own row, read-only when mayEdit is false, worded error on a refusal,
// tiers sent ascending (S11), and disabledReason disabling both buttons with that title (S12).
import { afterEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient, ApiError } from '../../../shared/api/client';
import { setLang } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import { makePricingDisplayDto } from '../../../shared/testing/fixtures/pricing_board';
import { makeRateDisplaySaveOutput } from '../../../shared/testing/fixtures/rate_display_save';
import { boardFetch } from './__fixtures__/pricing-board.fixture';
import { DISPLAY_FEATURED_MAX, ROUTE_PRICING_DISPLAY } from './pricing-labels';
import { DisplaySettings } from './DisplaySettings';

const CURRENCIES = ['USD', 'EUR', 'JPY', 'GBP', 'CNY', 'SGD', 'KRW', 'AUD', 'MYR', 'HKD', 'CHF', 'TWD'];

let host: HTMLDivElement | undefined;
let dispose: (() => void) | undefined;
function unmount() {
  dispose?.();
  dispose = undefined;
  host?.remove();
  host = undefined;
}
afterEach(() => {
  unmount();
  setLang('th');
});

async function flush() {
  for (let i = 0; i < 8; i++) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}

function mount(
  display: Parameters<typeof makePricingDisplayDto>[0],
  opts?: {
    mayEdit?: boolean;
    disabledReason?: string;
    command?: (path: string, body: unknown) => { status: number; body: unknown };
  },
) {
  const onSaved = vi.fn();
  const defaultCommand = (path: string, _body: unknown) =>
    path === ROUTE_PRICING_DISPLAY
      ? { status: 200, body: makeRateDisplaySaveOutput({ saved: true }) }
      : { status: 404, body: {} };
  const server = boardFetch(undefined, opts?.command ?? defaultCommand);
  const api = new ApiClient(server.fn as unknown as typeof fetch);
  const value = makePricingDisplayDto(display);
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(
    () => (
      <DisplaySettings
        api={api}
        branch="B01"
        value={value}
        currencies={CURRENCIES}
        mayEdit={opts?.mayEdit ?? true}
        disabledReason={opts?.disabledReason}
        onSaved={onSaved}
      />
    ),
    host,
  );
  const posts = () => server.calls.filter((c) => c.method === 'POST' && c.path === ROUTE_PRICING_DISPLAY);
  return { server, onSaved, posts };
}

const S1_VALUE = { tiers: [1, 3], featured: ['USD', 'EUR'], highlight: true, allMarket: false, from: 'MAIN' };

const q = <T extends Element>(sel: string) => host!.querySelector<T>(sel);
const tierBox = (n: number) => q<HTMLInputElement>(`input[name="tier"][value="${n}"]`);
const featuredCodes = () =>
  Array.from(host!.querySelectorAll<HTMLButtonElement>('button[name="featured-remove"][data-code]')).map(
    (b) => b.dataset.code,
  );
const addSelect = () => q<HTMLSelectElement>('select[name="featured-add"]');
const addOptions = () =>
  Array.from(addSelect()!.options)
    .map((o) => o.value)
    .filter((v) => v !== '');
function pick(code: string) {
  const sel = addSelect()!;
  sel.value = code;
  sel.dispatchEvent(new Event('change', { bubbles: true }));
}
function save() {
  const btn = q<HTMLButtonElement>('button[name="save-display"]');
  expect(btn, 'button[name="save-display"]').not.toBeNull();
  btn!.click();
}

describe('DisplaySettings', () => {
  it('S1 shows tiers 1 and 3 checked, featured USD then EUR, highlight checked, allMarket unchecked, "ใช้ของ MAIN"', () => {
    mount(S1_VALUE);
    for (const n of [1, 2, 3, 4]) {
      expect(tierBox(n), `input[name="tier"][value="${n}"]`).not.toBeNull();
    }
    expect([1, 2, 3, 4].map((n) => tierBox(n)!.checked)).toEqual([true, false, true, false]);
    expect(featuredCodes()).toEqual(['USD', 'EUR']);
    const highlight = q<HTMLInputElement>('input[name="highlight"]');
    const allMarket = q<HTMLInputElement>('input[name="allMarket"]');
    expect(highlight, 'input[name="highlight"]').not.toBeNull();
    expect(allMarket, 'input[name="allMarket"]').not.toBeNull();
    expect(highlight!.checked).toBe(true);
    expect(allMarket!.checked).toBe(false);
    expect(host!.textContent).toContain('ใช้ของ MAIN');
  });

  it('S2 uncheck tier 3, check tier 2, save: one keyed POST /api/pricing/display with all four fields', async () => {
    const { posts } = mount(S1_VALUE);
    expect(tierBox(3), 'input[name="tier"][value="3"]').not.toBeNull();
    tierBox(3)!.click();
    tierBox(2)!.click();
    save();
    await flush();

    const sent = posts();
    expect(sent).toHaveLength(1);
    expect(sent[0].path).toBe('/api/pricing/display');
    expect(sent[0].body).toEqual({ tiers: [1, 2], featured: ['USD', 'EUR'], highlight: true, allMarket: false });
    expect(sent[0].key, 'Idempotency-Key header').toBeTruthy();
  });

  it('S3 uncheck every tier, save: "เลือกอย่างน้อย 1 ขั้น" and no POST', async () => {
    const { posts } = mount(S1_VALUE);
    expect(tierBox(1), 'input[name="tier"][value="1"]').not.toBeNull();
    tierBox(1)!.click();
    tierBox(3)!.click();
    save();
    await flush();

    expect(host!.textContent).toContain('เลือกอย่างน้อย 1 ขั้น');
    expect(posts()).toHaveLength(0);
  });

  it('S4 add JPY: featured becomes USD, EUR, JPY and the add select no longer lists JPY', async () => {
    mount(S1_VALUE);
    expect(addSelect(), 'select[name="featured-add"]').not.toBeNull();
    expect(addOptions()).toContain('JPY');
    expect(addOptions()).not.toContain('USD');
    expect(addOptions()).not.toContain('EUR');

    pick('JPY');
    await flush();

    expect(featuredCodes()).toEqual(['USD', 'EUR', 'JPY']);
    expect(addOptions()).not.toContain('JPY');
  });

  it('S5 with 10 featured the add select is disabled', async () => {
    expect(DISPLAY_FEATURED_MAX).toBe(10);
    // 10 featured from the start.
    mount({ ...S1_VALUE, featured: CURRENCIES.slice(0, 10) });
    expect(addSelect(), 'select[name="featured-add"]').not.toBeNull();
    expect(addSelect()!.disabled).toBe(true);
    unmount();

    // 9 featured: still open; picking the 10th closes it.
    mount({ ...S1_VALUE, featured: CURRENCIES.slice(0, 9) });
    expect(addSelect()!.disabled).toBe(false);
    pick(CURRENCIES[9]);
    await flush();
    expect(featuredCodes()).toHaveLength(10);
    expect(addSelect()!.disabled).toBe(true);
  });

  it('S6 remove USD, save: featured ["EUR"]', async () => {
    const { posts } = mount(S1_VALUE);
    const remove = q<HTMLButtonElement>('button[name="featured-remove"][data-code="USD"]');
    expect(remove, 'button[name="featured-remove"][data-code="USD"]').not.toBeNull();
    remove!.click();
    await flush();
    save();
    await flush();

    const sent = posts();
    expect(sent).toHaveLength(1);
    expect((sent[0].body as { featured?: unknown }).featured).toEqual(['EUR']);
  });

  it('S7 from "branch", press clear: one POST whose body is exactly {clear: true}', async () => {
    const { posts } = mount({ ...S1_VALUE, from: 'branch' });
    const clear = q<HTMLButtonElement>('button[name="clear-display"]');
    expect(clear, 'button[name="clear-display"]').not.toBeNull();
    clear!.click();
    await flush();

    const sent = posts();
    expect(sent).toHaveLength(1);
    expect(sent[0].body).toStrictEqual({ clear: true });
  });

  it('S8 from "default": no clear button', () => {
    mount({ ...S1_VALUE, from: 'default' });
    // The form itself is there (so the missing clear button is not just an empty render).
    expect(q('button[name="save-display"]'), 'button[name="save-display"]').not.toBeNull();
    expect(q('button[name="clear-display"]')).toBeNull();
  });

  it('S9 mayEdit false: inputs disabled, no save or clear button', () => {
    mount({ ...S1_VALUE, from: 'branch' }, { mayEdit: false });
    const controls = Array.from(
      host!.querySelectorAll<HTMLInputElement | HTMLSelectElement | HTMLButtonElement>(
        'input, select, button[name="featured-remove"]',
      ),
    );
    // 4 tiers + highlight + allMarket at least.
    expect(controls.length).toBeGreaterThanOrEqual(6);
    for (const c of controls) {
      expect(c.disabled, `${c.tagName} name=${c.getAttribute('name')} disabled`).toBe(true);
    }
    expect(q('button[name="save-display"]')).toBeNull();
    expect(q('button[name="clear-display"]')).toBeNull();
  });

  it('S10 a 400 shows the worded error and does not call onSaved; a 200 calls onSaved', async () => {
    const refusal = { error: { code: 'validationFailed', message: 'tiers must hold 1 to 4 tiers' } };
    const refused = mount(S1_VALUE, { command: () => ({ status: 400, body: refusal }) });
    save();
    await flush();
    expect(refused.posts()).toHaveLength(1);
    const worded = wordedError(new ApiError({ ...refusal.error, status: 400 }));
    expect(worded.length).toBeGreaterThan(0);
    expect(host!.textContent).toContain(worded);
    expect(refused.onSaved).not.toHaveBeenCalled();
    unmount();

    const ok = mount(S1_VALUE);
    save();
    await flush();
    expect(ok.posts()).toHaveLength(1);
    expect(ok.onSaved).toHaveBeenCalledTimes(1);
    expect(ok.onSaved).toHaveBeenCalledWith(makeRateDisplaySaveOutput({ saved: true }));
  });

  it('S11 value.tiers [3], check tier 1, save: body.tiers is exactly [1, 3]', async () => {
    const { posts } = mount({ ...S1_VALUE, tiers: [3] });
    expect(tierBox(1), 'input[name="tier"][value="1"]').not.toBeNull();
    tierBox(1)!.click();
    save();
    await flush();

    const sent = posts();
    expect(sent).toHaveLength(1);
    expect((sent[0].body as { tiers?: unknown }).tiers).toStrictEqual([1, 3]);
  });

  it('S12 disabledReason "x": save-display and clear-display are disabled with title "x"', () => {
    mount({ ...S1_VALUE, from: 'branch' }, { disabledReason: 'x' });
    for (const name of ['save-display', 'clear-display']) {
      const btn = q<HTMLButtonElement>(`button[name="${name}"]`);
      expect(btn, `button[name="${name}"]`).not.toBeNull();
      expect(btn!.disabled, `${name} disabled`).toBe(true);
      expect(btn!.getAttribute('title'), `${name} title`).toBe('x');
    }
  });
});
