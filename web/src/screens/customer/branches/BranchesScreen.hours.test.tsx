// OL-6 wave B (cust-opening-hours, audit #564 issuecomment-6702970101): one row per day on the /app/branches card.
// Written before the implementation. Do not edit, delete or weaken; add tests in a file of your own.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { Fetch } from '../../../shared/api/client';
import type { PublicBranchDto } from '../../../shared/api/public/public_branches';
import { makeBranchServiceStatusData } from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { BranchesScreen } from './BranchesScreen';

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
  rules = parseCss(fs.readFileSync(here + 'branches.css', 'utf8'));
});

const BRANCH: PublicBranchDto = {
  id: 'br-1',
  code: 'MAIN',
  name: 'Siam',
  address: null,
  openingHours: { mon: [{ open: '09:00', close: '12:00' }, { open: '13:00', close: '18:00' }] },
  timezone: 'Asia/Bangkok',
  bookingEnabled: true,
  queueOpen: true,
};

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

function server(branch: PublicBranchDto): Fetch {
  return vi.fn(async (input: string) => {
    const u = new URL(input, 'http://test.local');
    if (u.pathname === '/api/public/orgs/XC/branches') return json(200, { branches: [branch] });
    if (u.pathname === '/api/v3/views/branch_service_status') {
      return json(200, { version: 1, changed: true, data: makeBranchServiceStatusData({ branch: 'br-1' }) });
    }
    return json(404, {});
  });
}

async function flush(): Promise<void> {
  for (let i = 0; i < 8; i += 1) await new Promise((r) => setTimeout(r, 0));
}

describe('cust-opening-hours: /app/branches look', () => {
  it('OH-B1 the list has no bullets or indent', () => {
    expect(value('.xc-branch-hours-list', 'list-style')).toBe('none');
    expect(value('.xc-branch-hours-list', 'padding')).toBe('0');
    expect(value('.xc-branch-hours-list', 'margin')).toBe('0');
  });

  it('OH-B2 a row is two columns: day, hours', () => {
    expect(value('.xc-branch-hours-row', 'display')).toBe('grid');
    expect(value('.xc-branch-hours-row', 'grid-template-columns')).toBe('5.5rem 1fr');
    expect(value('.xc-branch-hours-row', 'gap')).toBe('0.5rem');
  });
});

describe('cust-opening-hours: /app/branches', () => {
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

  async function mount(branch: PublicBranchDto): Promise<void> {
    const fetchFn = server(branch);
    dispose = render(() => <BranchesScreen fetch={fetchFn} org="XC" />, host);
    await flush();
  }

  it('OH-B3 seven rows, Monday first; a two-interval day stays on one row', async () => {
    await mount(BRANCH);
    const rows = [...host.querySelectorAll('.xc-branch-hours .xc-branch-hours-row')];
    expect(rows).toHaveLength(7);
    expect(rows[0]!.textContent).toBe('จันทร์: 09:00 - 12:00, 13:00 - 18:00');
    expect(rows[6]!.textContent).toBe('อาทิตย์: ปิดทำการ');
    expect([...rows[0]!.children].map((c) => c.textContent)).toEqual(['จันทร์:', '09:00 - 12:00, 13:00 - 18:00']);
  });

  it('OH-B4 no opening hours on the branch: the no-hours line, no list', async () => {
    await mount({ ...BRANCH, openingHours: null });
    expect(host.querySelector('.xc-branch-hours-row')).toBeNull();
    expect(host.querySelector('.xc-branch-hours')?.textContent).toContain('ไม่มีข้อมูลเวลาทำการ');
  });
});
