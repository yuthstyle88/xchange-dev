// OL-6 wave B admin (audit #564 issuecomment-6702970101, row 028): the count-difference limit screen's look. The first
// card no longer repeats the page title, the OD help has room above its table, the levels table scrolls inside its card,
// and its inputs are 16px like every other field. Written before the fix by claude/94389a6b. Do not edit, delete or
// weaken; add your own elsewhere.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { sharedSession } from '../../../shared/session/session';
import { makeCashLimitsData, makeMe } from '../../../shared/testing/fixtures';
import { CashLimits } from './CashLimits';
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
  css = fs.readFileSync(here + 'cash-limits.css', 'utf8');
});

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

beforeEach(async () => {
  setLang('th');
  sharedSession().updateMe(makeMe({ permissions: ['CashLimitsManage'] }));
  vi.stubGlobal(
    'fetch',
    vi.fn(
      async () =>
        new Response(
          JSON.stringify({
            version: 1,
            changed: true,
            data: makeCashLimitsData({
              branch: 'MAIN',
              countDifferenceLimitThb: '100.00',
              drawerLowLevels: [{ currency: 'USD', lowLevel: '50' }],
            }),
          }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        ),
    ),
  );
  host = document.createElement('div');
  document.body.appendChild(host);
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-11-07', to: '2026-11-07' });
  const [shown] = createSignal(true);
  dispose = render(() => <CashLimits branch={branch} period={period} shown={shown} />, host);
  for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.unstubAllGlobals();
});

describe('cash-limits look', () => {
  it('CL-1 no card repeats the page title or eyebrow (the shell heading shows them)', () => {
    const titles = [...host.querySelectorAll('.card-title')].map((e) => e.textContent);
    expect(titles).not.toContain(th.title);
    expect(host.querySelector('.card .eyebrow')).toBeNull();
  });

  it('CL-2 the OD card keeps its own heading', () => {
    const titles = [...host.querySelectorAll('.card-title')].map((e) => e.textContent);
    expect(titles).toContain(th.levelsTitle);
  });

  it('CL-3 the help text has room above the levels form', () => {
    expect(decls('.xc-admin .xc-content [data-section="drawer-low-levels"] form')?.get('margin-top')).toBe('16px');
  });

  it('CL-4 the levels table scrolls inside its card', () => {
    expect(host.querySelector('[data-section="drawer-low-levels"] .table-scroll table')).not.toBeNull();
  });

  it('CL-5 table inputs are 16px, with a selector that beats the admin field rule', () => {
    expect(decls('.xc-admin .xc-content .cash-limits table input')?.get('font-size')).toBe('16px');
  });
});
