// OL-6 wave B admin-denomination-groups (audit #564 issuecomment-6702970101, shot 027): the page was one full-width
// select on the grey page, with no card and nothing saying what to do; its blank choice repeated the label
// ("สกุลเงิน" / "สกุลเงิน").
// - The picker sits in a white card (`div.card.denomination-groups-pick-card`).
// - The blank choice reads "เลือกสกุลเงิน" / "Choose a currency".
// - Until a currency is chosen, a hint under the select says what the page shows; it goes once one is chosen.
// Written before the fix by claude/bbe9ce8b for the OL-6 sub-controller claude/1cc51615. Do not edit, delete or weaken.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { Dialog, closeDialog } from '../parts/Dialog';
import { Toast } from '../parts/toast';
import { QUEUE_CARD_PATH, queueCardUnchanged } from '../../../shared/testing/page-wide';
import { menu } from './menu';
import { en, th } from './i18n';

const HINT_TH = 'เลือกสกุลเงินเพื่อดูกลุ่มธนบัตรและการจับคู่แถวบนป้ายเรท';
const HINT_EN = 'Choose a currency to see its banknote groups and rate-board rows';

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

async function fakeFetch(input: string): Promise<Response> {
  const path = new URL(input, 'http://localhost').pathname;
  if (path === '/api/v3/views/me') {
    return json(200, {
      version: 1,
      changed: true,
      data: { kind: 'owner', id: 'owner-1', role: 'OWNER', branch: 'MAIN', name: 'Owner', username: 'owner' },
    });
  }
  if (path === QUEUE_CARD_PATH) return json(200, queueCardUnchanged());
  if (path === '/api/v3/views/admin_denomination_groups') {
    return json(200, {
      version: 1,
      changed: true,
      data: {
        provider: 'RATE_BOARD',
        currencies: [
          { currency: 'EUR', groups: [{ group: 'ALL', instrumentKey: null, removable: false }] },
          { currency: 'USD', groups: [{ group: 'ALL', instrumentKey: 'USD', removable: false }] },
        ],
      },
    });
  }
  return json(500, { error: { code: 'serverError', message: `unexpected ${path}` } });
}

async function settle(): Promise<void> {
  for (let round = 0; round < 4; round++) {
    for (let i = 0; i < 5; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
}

let dispose: (() => void) | undefined;
let root: HTMLElement | undefined;

async function mount(): Promise<HTMLElement> {
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-10-11', to: '2026-10-11' });
  const [shown] = createSignal(true);
  const Component = menu.Component;
  const el = document.createElement('div');
  root = el;
  document.body.appendChild(el);
  dispose = render(
    () =>
      (
        <>
          <Component branch={branch} period={period} shown={shown} />
          <Dialog />
          <Toast />
        </>
      ) as JSX.Element,
    el,
  );
  await settle();
  return el;
}

beforeEach(() => {
  vi.stubGlobal('fetch', vi.fn(fakeFetch));
  location.hash = '#denomination-groups';
  setLang('th');
});

afterEach(async () => {
  closeDialog();
  dispose?.();
  dispose = undefined;
  root?.remove();
  root = undefined;
  vi.unstubAllGlobals();
  location.hash = '';
  setLang('th');
  await settle();
});

const text = (el: Element | null | undefined) => (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
const picker = (el: ParentNode) => el.querySelector<HTMLSelectElement>('.denomination-groups-currency-picker select')!;
const hint = (el: ParentNode) => el.querySelector<HTMLElement>('.denomination-groups-hint');

describe('admin-denomination-groups: the currency picker', () => {
  it('DG-1 the picker sits in a white card: div.card.denomination-groups-pick-card', async () => {
    const el = await mount();
    const card = picker(el).closest('.card');
    expect(card, 'the select is inside a .card').not.toBeNull();
    expect(card!.tagName).toBe('DIV');
    expect(card!.classList.contains('denomination-groups-pick-card')).toBe(true);
  });

  it('DG-2 the blank choice reads เลือกสกุลเงิน, not the label again', async () => {
    const el = await mount();
    const first = picker(el).options[0]!;
    expect(first.value).toBe('');
    expect(text(first)).toBe('เลือกสกุลเงิน');
    expect(text(first)).not.toBe(th.currency);
  });

  it('DG-3 before a currency is chosen, a hint inside the card says what the page shows', async () => {
    const el = await mount();
    expect(text(hint(el))).toBe(HINT_TH);
    expect(hint(el)!.closest('.denomination-groups-pick-card')).not.toBeNull();
  });

  it('DG-4 once a currency is chosen the hint goes and the groups show', async () => {
    const el = await mount();
    const s = picker(el);
    s.value = 'USD';
    s.dispatchEvent(new Event('input', { bubbles: true }));
    s.dispatchEvent(new Event('change', { bubbles: true }));
    await settle();
    expect(hint(el)).toBeNull();
    expect(el.querySelector('.denomination-groups-list')).not.toBeNull();
  });

  it('DG-5 the two keys exist in both tables (th / en)', () => {
    const T = th as Record<string, string>;
    const E = en as Record<string, string>;
    expect(T.chooseCurrency).toBe('เลือกสกุลเงิน');
    expect(E.chooseCurrency).toBe('Choose a currency');
    expect(T.pickHint).toBe(HINT_TH);
    expect(E.pickHint).toBe(HINT_EN);
  });
});

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

function decls(css: string, selector: string): Map<string, string> | undefined {
  const stripped = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(stripped)) !== null) {
    if (m[1]!.trim() !== selector) continue;
    const out = new Map<string, string>();
    for (const d of m[2]!.split(';')) {
      const i = d.indexOf(':');
      if (i > 0) out.set(d.slice(0, i).trim().toLowerCase(), d.slice(i + 1).trim());
    }
    return out;
  }
  return undefined;
}

describe('admin-denomination-groups: denomination-groups.css', () => {
  let css = '';
  beforeAll(async () => {
    const specifier = 'node:fs';
    const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
    // Resolve next to this file, not process.cwd(): main's web job runs vitest with --root web from the repo root.
    const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
    css = fs.readFileSync(here + 'denomination-groups.css', 'utf8');
  });

  it("DG-6 the pick card stacks label and hint (column, 8px) and drops card.css's bottom margin", () => {
    const d = decls(css, '.xc-admin .xc-content .denomination-groups-pick-card');
    expect(d, 'rule `.xc-admin .xc-content .denomination-groups-pick-card`').toBeDefined();
    expect(d!.get('display')).toBe('flex');
    expect(d!.get('flex-direction')).toBe('column');
    expect(d!.get('gap')).toBe('8px');
    expect(d!.get('margin-bottom')).toBe('0');
  });

  it('DG-7 the hint is 13px muted with no margin', () => {
    const d = decls(css, '.xc-admin .xc-content .denomination-groups-hint');
    expect(d, 'rule `.xc-admin .xc-content .denomination-groups-hint`').toBeDefined();
    expect(d!.get('margin')).toBe('0');
    expect(d!.get('font-size')).toBe('13px');
    expect(d!.get('color')).toBe('var(--xc-muted)');
  });
});
