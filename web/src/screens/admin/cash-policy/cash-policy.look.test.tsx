// OL-6 wave B admin (audit #564 issuecomment-6702970101, row 029): the transfer-policy screen's look. The banner shows a
// short version (the data has no other label), the card no longer repeats the page title, the limit header sits over its
// input, and the inputs are 16px. Written before the fix by claude/94389a6b. Do not edit, delete or weaken; add your own
// elsewhere.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { sharedSession } from '../../../shared/session/session';
import {
  makeAdminCashData,
  makeAdminCashStockRow,
  makeCashTransferPolicyData,
  makeMe,
} from '../../../shared/testing/fixtures';
import { CashPolicy } from './CashPolicy';
import { th } from './i18n';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

let css = '';

function decls(selector: string): Map<string, string> | undefined {
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

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  css = fs.readFileSync(here + 'cash-policy.css', 'utf8');
});

const UUID = '50587ce9-5c11-456b-afda-23ca4653e43c';

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function json(body: unknown): Response {
  return new Response(JSON.stringify(body), { status: 200, headers: { 'content-type': 'application/json' } });
}

async function mount(version: string): Promise<void> {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) => {
      const path = new URL(String(input), 'http://xc.test').pathname;
      if (path === '/api/v3/views/admin_cash') {
        return json({
          version: 1,
          changed: true,
          data: makeAdminCashData({
            openShifts: [],
            stock: [makeAdminCashStockRow({ code: 'THB' })],
            stats: null,
            currencies: ['THB'],
            lastCounted: null,
          }),
        });
      }
      return json({
        version: 1,
        changed: true,
        data: makeCashTransferPolicyData({ version, limits: { THB: '20000.00' } }),
      });
    }),
  );
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-11-07', to: '2026-11-07' });
  const [shown] = createSignal(true);
  dispose = render(() => <CashPolicy branch={branch} period={period} shown={shown} />, host);
  for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
}

beforeEach(() => {
  setLang('th');
  sharedSession().updateMe(makeMe({ permissions: ['CashTransferPolicySet'] }));
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.unstubAllGlobals();
});

describe('cash-policy version banner', () => {
  it('CP-1 a UUID version shows its first 8 characters, the whole id on hover', async () => {
    await mount(UUID);
    const strong = host.querySelector('.stock-banner strong')!;
    expect(strong.textContent).toContain('ฉบับ 50587ce9 ·');
    expect(strong.textContent).not.toContain('-5c11-');
    expect(strong.getAttribute('title')).toBe(UUID);
  });

  it('CP-2 a version that is not a UUID is shown as sent', async () => {
    await mount('v-0001');
    expect(host.querySelector('.stock-banner strong')?.textContent).toContain('ฉบับ v-0001 ·');
  });
});

describe('cash-policy card and table', () => {
  it('CP-3 the card does not repeat the page title or eyebrow', async () => {
    await mount(UUID);
    expect(host.querySelector('.card .eyebrow')).toBeNull();
    expect([...host.querySelectorAll('.card-title')].map((e) => e.textContent)).not.toContain(th.title);
  });

  it('CP-4 the limit header and cell are left-aligned, over the left-aligned input', async () => {
    await mount(UUID);
    expect(host.querySelector('.cash-policy-table th.num')).toBeNull();
    expect(host.querySelector('.cash-policy-table td.num')).toBeNull();
  });

  it('CP-5 table inputs are 16px, with a selector that beats the admin field rule', () => {
    expect(decls('.xc-admin .xc-content .admin-cash-policy-screen .cash-policy-table input')?.get('font-size')).toBe('16px');
  });

  it('CP-6 the table rule no longer sets its own font size, which the admin table rule always beat', () => {
    expect(decls('.xc-admin .xc-content .cash-policy-table')?.has('font-size')).toBe(false);
  });
});
