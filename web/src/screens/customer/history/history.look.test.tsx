// OL-6 wave B (cust-history, audit #564 issuecomment-6702970101): page title; empty state is a card with a way forward.
// Written before the implementation. Do not edit, delete or weaken; add tests in a file of your own.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { makeMyTransactionsData } from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { HistoryScreen } from './HistoryScreen';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

type Rule = { media: string; selector: string; decls: Map<string, string> };

function parseCss(text: string): Rule[] {
  const src = text.replace(/\/\*[\s\S]*?\*\//g, '');
  const out: Rule[] = [];
  const norm = (s: string) => s.replace(/\s+/g, '');
  const normSel = (s: string) => s.trim().replace(/\s+/g, ' ').replace(/\s*([>+~(),])\s*/g, '$1');
  const block = (body: string, media: string) => {
    const re = /([^{}]+)\{([^{}]*)\}/g;
    let m: RegExpExecArray | null;
    while ((m = re.exec(body)) !== null) {
      const decls = new Map<string, string>();
      for (const d of m[2]!.split(';')) {
        const k = d.indexOf(':');
        if (k > 0) decls.set(d.slice(0, k).trim().toLowerCase(), d.slice(k + 1).trim().replace(/\s+/g, ' '));
      }
      for (const sel of m[1]!.split(',')) out.push({ media, selector: normSel(sel), decls });
    }
  };
  let rest = '';
  let i = 0;
  while (i < src.length) {
    const at = src.indexOf('@media', i);
    if (at < 0) {
      rest += src.slice(i);
      break;
    }
    rest += src.slice(i, at);
    const open = src.indexOf('{', at);
    let depth = 1;
    let j = open + 1;
    while (j < src.length && depth > 0) {
      if (src[j] === '{') depth += 1;
      if (src[j] === '}') depth -= 1;
      j += 1;
    }
    block(src.slice(open + 1, j - 1), norm(src.slice(at + 6, open)));
    i = j;
  }
  block(rest, '');
  return out;
}

let rules: Rule[] = [];

/** The value the stylesheet's last rule for this exact selector gives the property, inside `media` when named. */
function value(selector: string, prop: string, media = ''): string | undefined {
  const m = media.replace(/\s+/g, '');
  for (let i = rules.length - 1; i >= 0; i -= 1) {
    const r = rules[i]!;
    if (r.selector === selector && r.media === m && r.decls.has(prop)) return r.decls.get(prop);
  }
  return undefined;
}

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  rules = parseCss(fs.readFileSync(here + 'history.css', 'utf8'));
});

async function flush(): Promise<void> {
  for (let i = 0; i < 6; i++) await new Promise((resolve) => setTimeout(resolve, 0));
}

describe('cust-history: look', () => {
  it('HI-1 the title has the customer page title look', () => {
    expect(value('.xc-history-title', 'margin')).toBe('0 0 1rem');
    expect(value('.xc-history-title', 'font-size')).toBe('1.5rem');
    expect(value('.xc-history-title', 'font-weight')).toBe('700');
  });

  it('HI-2 the empty state is a card', () => {
    expect(value('.xc-history-empty', 'background')).toBe('var(--xc-card)');
    expect(value('.xc-history-empty', 'border-radius')).toBe('var(--xc-radius-card)');
    expect(value('.xc-history-empty', 'box-shadow')).toBe('var(--xc-shadow-card)');
    expect(value('.xc-history-empty', 'display')).toBe('flex');
    expect(value('.xc-history-empty', 'flex-direction')).toBe('column');
    expect(value('.xc-history-empty', 'gap')).toBe('12px');
  });
});

describe('cust-history: screen', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.restoreAllMocks();
  });

  async function mountEmpty(): Promise<void> {
    const body = JSON.stringify({ version: 1, changed: true, data: makeMyTransactionsData({ transactions: [] }) });
    const fetchMock = vi.fn(async () => new Response(body, { status: 200, headers: { 'Content-Type': 'application/json' } }));
    dispose = render(() => <HistoryScreen fetch={fetchMock} />, host);
    await flush();
  }

  it('HI-3 the page has one h1 with the title', async () => {
    await mountEmpty();
    expect(host.querySelector('h1.xc-history-title')?.textContent).toBe('ประวัติรายการ');
  });

  it('HI-4 the empty state keeps its message and links to booking', async () => {
    await mountEmpty();
    const empty = host.querySelector('.xc-history-empty');
    expect(empty?.textContent).toContain('ไม่มีรายการ');
    const cta = empty?.querySelector<HTMLAnchorElement>('a.xc-c-primary');
    expect(cta?.getAttribute('href')).toBe('/app/book');
    expect(cta?.textContent).toBe('จองคิวแลกเงินล่วงหน้า');
  });
});
