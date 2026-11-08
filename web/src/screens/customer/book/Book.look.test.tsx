// OL-6 wave B (cust-book, audit #564 issuecomment-6702970101): select placeholders, one no-slot message, upright hint, page gutter.
// Written before the implementation. Do not edit, delete or weaken; add tests in a file of your own.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import { warmModules } from '../../../shared/testing/warm-modules';
import { BookScreen } from './Book';

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
  rules = parseCss(fs.readFileSync(here + 'book.css', 'utf8'));
});

const BRANCH_ID = 'XC-B01';
const BRANCHES = {
  branches: [{ id: BRANCH_ID, code: BRANCH_ID, name: 'สาขาสยามพารากอน', timezone: 'Asia/Bangkok', bookingEnabled: true }],
};

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

async function flush(): Promise<void> {
  for (let i = 0; i < 15; i++) await new Promise((r) => setTimeout(r, 0));
}

describe('cust-book: look', () => {
  it('BK-1 the screen gutter is 1rem', () => {
    expect(value('.xc-book-screen', 'padding')).toBe('1rem 1rem 2rem');
  });

  it('BK-2 the empty-state hints are upright', () => {
    for (const sel of ['.xc-book-empty', '.xc-book-branches-empty', '.xc-book-slots-empty']) {
      expect(value(sel, 'font-style'), sel).toBeUndefined();
    }
  });
});

describe('cust-book: the selects say what to do', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  const fetchStub = vi.fn(async (input: string): Promise<Response> => {
    const u = new URL(input, 'http://xc.test');
    if (u.pathname === '/api/public/host') return json(200, { org: 'XC', env: 'test' });
    if (u.pathname.startsWith('/api/public/orgs/')) return json(200, BRANCHES);
    if (u.pathname === '/api/v3/views/booking_slots') return json(200, { version: 1, changed: true, data: { slots: [] } });
    return json(404, { error: { code: 'notFound', message: u.pathname } });
  });

  beforeAll(() => warmModules(() => import('./Book')), 60_000);

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    window.history.replaceState({}, '', '/app/book');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.restoreAllMocks();
  });

  // Round 13 (BK-CP, #564): the placeholder is the currency's own "choose a branch first", not the branch prompt.
  it('BK-3 before a branch is chosen the currency select says to choose a branch first', async () => {
    dispose = render(() => <BookScreen fetch={fetchStub} />, host);
    await flush();
    const options = host.querySelectorAll<HTMLOptionElement>('select[name="currency"] option');
    expect(options.length).toBe(1);
    expect(options[0]!.value).toBe('');
    expect(options[0]!.textContent).toBe('เลือกสาขาก่อน');
  });

  it('BK-4 a day with no slots says so once: the status line, not the select as well', async () => {
    dispose = render(() => <BookScreen fetch={fetchStub} />, host);
    await flush();
    expect(host.querySelector('.xc-book-slots-empty')?.textContent).toBe('ไม่มีช่วงเวลาที่เปิดรับจอง');
    expect(host.querySelector('select[name="slotId"] option[value=""]')?.textContent).toBe('เลือกช่วงเวลา…');
  });
});
