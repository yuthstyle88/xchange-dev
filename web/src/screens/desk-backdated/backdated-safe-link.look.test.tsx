// OL-8 backdated-safe-link (verification r12, "Seen outside the 138 rows"): on /desk/backdated the SAFE card's
// "เติมเงิน SAFE" is a browser-default link (rgb(0,0,238), underlined). It must be the theme's secondary action:
// `.xc-btn .xc-btn--secondary` from shared/ui/xc-theme.css, without the link underline.
// Written before the fix by claude/883e8664 (pane claude-3). Do not edit, delete or weaken.
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiClient } from '../../shared/api/client';
import { setLang } from '../../shared/i18n';
import { SafeBalance } from './SafeBalance';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

async function readHere(file: string): Promise<string> {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  return fs.readFileSync(here + file, 'utf8');
}

function decls(css: string, selector: string): Map<string, string> | undefined {
  const re = /([^{}]+)\{([^{}]*)\}/g;
  const plain = css.replace(/\/\*[\s\S]*?\*\//g, '');
  let m: RegExpExecArray | null;
  while ((m = re.exec(plain)) !== null) {
    if (!m[1]!.split(',').map((s) => s.trim()).includes(selector)) continue;
    const out = new Map<string, string>();
    for (const d of m[2]!.split(';')) {
      const i = d.indexOf(':');
      if (i > 0) out.set(d.slice(0, i).trim().toLowerCase(), d.slice(i + 1).trim());
    }
    return out;
  }
  return undefined;
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let balances: Array<{ currency: string; amount: string }>;

function api(): ApiClient {
  return new ApiClient(async (url) => {
    const u = new URL(url, 'http://desk.test');
    if (u.pathname === '/api/v3/views/backdate_safe') {
      return new Response(JSON.stringify({ version: 1, changed: true, data: { balances } }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });
    }
    return new Response(JSON.stringify({ version: 1, changed: false }), { status: 200 });
  });
}

async function settle(): Promise<void> {
  for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 10));
}

async function mount(): Promise<HTMLAnchorElement> {
  const [refresh] = createSignal(0);
  const client = api();
  dispose = render(() => <SafeBalance api={client} branch="MAIN" refresh={refresh} />, host);
  await settle();
  const link = host.querySelector<HTMLAnchorElement>('[data-testid="safe-transfer-link"]');
  expect(link, 'the SAFE top-up link').not.toBeNull();
  return link!;
}

beforeEach(() => {
  setLang('th');
  balances = [{ currency: 'THB', amount: '150000.00' }];
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
});

describe('OL-8 backdated-safe-link: the SAFE top-up link is the theme secondary action', () => {
  it('BSL-1 the link carries xc-btn + xc-btn--secondary and still goes to /staff with its text', async () => {
    const link = await mount();
    expect(link.tagName).toBe('A');
    expect(link.classList.contains('xc-btn'), 'class xc-btn').toBe(true);
    expect(link.classList.contains('xc-btn--secondary'), 'class xc-btn--secondary').toBe(true);
    expect(link.getAttribute('href')).toBe('/staff');
    expect(link.textContent?.trim()).toBe('เติมเงิน SAFE');
  });

  it('BSL-2 the empty SAFE state shows the same styled link', async () => {
    balances = [];
    const link = await mount();
    expect(host.querySelector('[data-testid="safe-empty"]'), 'the empty line').not.toBeNull();
    expect(link.classList.contains('xc-btn'), 'class xc-btn').toBe(true);
    expect(link.classList.contains('xc-btn--secondary'), 'class xc-btn--secondary').toBe(true);
  });

  it('BSL-3 backdated.css drops the link underline on buttons in the summary card', async () => {
    const css = await readHere('backdated.css');
    const rule = decls(css, '.backdated-summary .xc-btn');
    expect(rule, 'rule `.backdated-summary .xc-btn` in backdated.css').toBeDefined();
    expect(rule!.get('text-decoration')).toBe('none');
  });
});
