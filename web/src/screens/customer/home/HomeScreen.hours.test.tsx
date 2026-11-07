// OL-6 wave B (cust-opening-hours, audit #564 issuecomment-6702970101): one row per day: openingHoursRows and the /app card.
// Written before the implementation. Do not edit, delete or weaken; add tests in a file of your own.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { PublicBranchDto, PublicBranchesData } from '../../../shared/api/public/public_branches';
import { makePublishedRatesData } from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { HomeScreen } from './HomeScreen';

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
  rules = parseCss(fs.readFileSync(here + 'home.css', 'utf8'));
});

const BRANCH: PublicBranchDto = {
  id: 'br-1',
  code: 'MAIN',
  name: 'Siam',
  address: null,
  openingHours: { mon: [{ open: '09:00', close: '12:00' }, { open: '13:00', close: '18:00' }], tue: [{ open: '10:00', close: '20:00' }] },
  timezone: 'Asia/Bangkok',
  bookingEnabled: true,
  queueOpen: true,
};

function server(branch: PublicBranchDto) {
  return vi.fn(async (url: string): Promise<Response> => {
    if (url === '/api/public/host') return new Response(JSON.stringify({ org: 'XC' }), { status: 200 });
    if (url === '/api/public/orgs/XC/branches') {
      const payload: PublicBranchesData = { branches: [branch] };
      return new Response(JSON.stringify(payload), { status: 200 });
    }
    if (url.endsWith('/rates')) return new Response(JSON.stringify(makePublishedRatesData({ lines: [] })), { status: 200 });
    return new Response('Not Found', { status: 404 });
  });
}

async function settle(): Promise<void> {
  for (let i = 0; i < 10; i += 1) await new Promise((r) => setTimeout(r, 0));
}

describe('cust-opening-hours: /app look', () => {
  it('OH-H1 the list has no bullets or indent', () => {
    expect(value('.xc-home-hours', 'list-style')).toBe('none');
    expect(value('.xc-home-hours', 'padding')).toBe('0');
    expect(value('.xc-home-hours', 'margin')).toBe('0.25rem 0 0');
  });

  it('OH-H2 a row is two columns: day, hours', () => {
    expect(value('.xc-home-hours-row', 'display')).toBe('grid');
    expect(value('.xc-home-hours-row', 'grid-template-columns')).toBe('5.5rem 1fr');
    expect(value('.xc-home-hours-row', 'gap')).toBe('0.5rem');
  });
});

describe('cust-opening-hours: /app', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    window.localStorage.clear();
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.restoreAllMocks();
  });

  async function mount(branch: PublicBranchDto): Promise<void> {
    const fetchFn = server(branch);
    dispose = render(() => <HomeScreen fetch={fetchFn} org="XC" />, host);
    await settle();
  }

  it('OH-H3 seven rows, Monday first, each day on its own row', async () => {
    await mount(BRANCH);
    const rows = [...host.querySelectorAll('.xc-home-branch-hours .xc-home-hours-row')];
    expect(rows.map((r) => r.textContent)).toEqual([
      'จันทร์: 09:00 - 12:00, 13:00 - 18:00',
      'อังคาร: 10:00 - 20:00',
      'พุธ: ปิดทำการ',
      'พฤหัสบดี: ปิดทำการ',
      'ศุกร์: ปิดทำการ',
      'เสาร์: ปิดทำการ',
      'อาทิตย์: ปิดทำการ',
    ]);
  });

  it('OH-H4 a row has two cells, for the two columns', async () => {
    await mount(BRANCH);
    const first = host.querySelector('.xc-home-hours-row')!;
    expect([...first.children].map((c) => c.textContent)).toEqual(['จันทร์:', '09:00 - 12:00, 13:00 - 18:00']);
  });

  it('OH-H5 no opening hours on the branch: no hours block', async () => {
    await mount({ ...BRANCH, openingHours: null });
    expect(host.querySelector('.xc-home-branch-name')?.textContent).toBe('Siam');
    expect(host.querySelector('.xc-home-branch-hours')).toBeNull();
  });
});
