// OL-6 wave B admin (audit #564 issuecomment-6702970101, row 039): the loyalty rules screen's look and its failed read.
// A failed read is a danger box with a retry, and the version tables are not drawn as if the screen had loaded. Every
// section is the shared card, tables scroll inside it and say when they are empty, and the draft form's grid comes from
// the screen's own stylesheet instead of campaigns.css leaking in. Written before the fix by claude/94389a6b. Do not
// edit, delete or weaken; add your own elsewhere.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { Loyalty } from './Loyalty';
import { en, th } from './i18n';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
  existsSync(file: string): boolean;
}

let css = '';
let source = '';

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
  css = fs.existsSync(here + 'loyalty.css') ? fs.readFileSync(here + 'loyalty.css', 'utf8') : '';
  source = fs.readFileSync(here + 'Loyalty.tsx', 'utf8');
});

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let failReads = 0;
let reads = 0;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) await new Promise((r) => setTimeout(r, 0));
}

async function mount(): Promise<void> {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) => {
      const u = new URL(String(input), 'http://xc.test');
      if (u.pathname === '/api/v3/views/loyalty_rule_versions') {
        reads++;
        if (failReads > 0) {
          failReads--;
          return json(503, { error: { code: 'serviceUnavailable', message: 'down' } });
        }
        return json(200, { pointVersions: [], tierVersions: [] });
      }
      return json(200, { version: 1, changed: false });
    }),
  );
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-11-07', to: '2026-11-07' });
  const [shown] = createSignal(true);
  dispose = render(() => <Loyalty branch={branch} period={period} shown={shown} />, host);
  await settle();
}

beforeEach(() => {
  setLang('th');
  failReads = 0;
  reads = 0;
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.unstubAllGlobals();
});

const versionTables = () => host.querySelectorAll('table');

describe('loyalty: a failed read', () => {
  it('LY-1 shows a danger box holding the alert and a retry button', async () => {
    failReads = 1;
    await mount();
    const box = host.querySelector('.loyalty-error');
    expect(box, 'the error box').not.toBeNull();
    expect(box!.querySelector('[role="alert"]')).not.toBeNull();
    expect(box!.querySelector('button[data-action="retry"]')?.textContent).toBe(th.retry);
    expect(th.retry).toBe('ลองอีกครั้ง');
    expect(en.retry).toBe('Try again');
  });

  it('LY-2 does not draw the version tables as if the screen had loaded', async () => {
    failReads = 1;
    await mount();
    expect(versionTables()).toHaveLength(0);
  });

  it('LY-3 retry reads again and, on success, shows the tables and drops the box', async () => {
    failReads = 1;
    await mount();
    host.querySelector<HTMLButtonElement>('button[data-action="retry"]')!.click();
    await settle();
    expect(reads).toBe(2);
    expect(host.querySelector('.loyalty-error')).toBeNull();
    expect(versionTables()).toHaveLength(2);
  });

  it('LY-4 the box uses the danger tokens', () => {
    const d = decls('.xc-admin .xc-content .loyalty-error');
    expect(d?.get('background')).toBe('var(--xc-danger-soft)');
    expect(d?.get('border')).toBe('1px solid var(--xc-danger)');
    expect(d?.get('color')).toBe('var(--xc-danger)');
    expect(d?.get('border-radius')).toBe('8px');
    expect(d?.get('display')).toBe('flex');
  });
});

describe('loyalty: a good read', () => {
  it('LY-5 the screen imports its own stylesheet and roots its rules in .loyalty-screen', async () => {
    await mount();
    expect(source).toContain("import './loyalty.css';");
    expect(host.querySelector('.loyalty-screen')).not.toBeNull();
    const grid = decls('.xc-admin .xc-content .loyalty-screen .xc-form-grid');
    expect(grid?.get('display')).toBe('grid');
    expect(grid?.get('grid-template-columns')).toBe('repeat(auto-fit, minmax(240px, 1fr))');
    expect(grid?.get('gap')).toBe('16px');
    const group = decls('.xc-admin .xc-content .loyalty-screen .xc-form-group');
    expect(group?.get('display')).toBe('flex');
    expect(group?.get('flex-direction')).toBe('column');
  });

  it('LY-6 the three sections are cards; both tables scroll inside theirs', async () => {
    await mount();
    const sections = [...host.querySelectorAll('.loyalty-screen > section')];
    expect(sections).toHaveLength(3);
    for (const s of sections) expect(s.classList.contains('card')).toBe(true);
    for (const t of versionTables()) expect(t.closest('.card .table-scroll')).not.toBeNull();
  });

  it('LY-7 an empty table says there is no version yet, across all its columns', async () => {
    await mount();
    const cells = [...host.querySelectorAll('tbody td[colspan]')];
    expect(cells.map((c) => c.getAttribute('colspan'))).toEqual(['10', '6']);
    for (const c of cells) expect(c.textContent).toBe(th.noVersions);
    expect(en.noVersions).toBe('No versions yet');
    expect(th.noVersions).toBe('ยังไม่มีเวอร์ชัน');
  });

  it('LY-8 save draft is the primary button, with room above it', async () => {
    await mount();
    expect(host.querySelector('button[data-action="save-draft"]')?.classList.contains('primary')).toBe(true);
    expect(decls('.xc-admin .xc-content .loyalty-screen [data-form="point-draft"] > button')?.get('margin-top')).toBe('16px');
  });

  it('LY-9 section headings have one size and one gap', () => {
    const d = decls('.xc-admin .xc-content .loyalty-screen h4');
    expect(d?.get('margin')).toBe('0 0 12px');
    expect(d?.get('font-size')).toBe('16px');
  });
});
