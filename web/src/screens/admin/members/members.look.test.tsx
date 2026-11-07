// OL-6 wave B admin (audit #564 issuecomment-6702970101, row 036): the members screen's look. Its sections are the
// shared white `.card`, the save row has room under the fields, and each step says what comes next: nothing defined
// yet, defined but no snapshot, and a snapshot with no matching member. There is no list of saved groups: no view
// returns one (only `segment_preview`), so that part of row 036 is not here. Written before the fix by
// claude/94389a6b. Do not edit, delete or weaken; add your own elsewhere.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { Members } from './Members';
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
  css = fs.existsSync(here + 'members.css') ? fs.readFileSync(here + 'members.css', 'utf8') : '';
  source = fs.readFileSync(here + 'Members.tsx', 'utf8');
});

function json(body: unknown): Response {
  return new Response(JSON.stringify(body), { status: 200, headers: { 'content-type': 'application/json' } });
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

beforeEach(async () => {
  setLang('th');
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const u = new URL(String(input), 'http://xc.test');
      const method = (init?.method ?? 'GET').toUpperCase();
      if (method === 'POST' && u.pathname === '/api/segments') return json({ id: 'seg-1', name: 'VIP', version: 1 });
      if (method === 'POST' && /^\/api\/segments\/[^/]+\/snapshots$/.test(u.pathname)) {
        return json({ id: 'snap-1', segmentId: 'seg-1', memberCount: 0 });
      }
      if (u.pathname === '/api/v3/views/segment_preview') {
        return json({
          version: 1,
          changed: true,
          data: { snapshotId: 'snap-1', memberCount: 0, distribution: { avgFrequency: 0, totalValueThb: '0.00' }, sample: [] },
        });
      }
      return json({ version: 1, changed: false });
    }),
  );
  host = document.createElement('div');
  document.body.appendChild(host);
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-11-07', to: '2026-11-07' });
  const [shown] = createSignal(true);
  dispose = render(() => <Members branch={branch} period={period} shown={shown} />, host);
  await settle();
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.unstubAllGlobals();
});

async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) await new Promise((r) => setTimeout(r, 0));
}

async function define(): Promise<void> {
  const name = host.querySelector<HTMLInputElement>('#segment-name')!;
  name.value = 'VIP';
  name.dispatchEvent(new Event('input', { bubbles: true }));
  host.querySelector<HTMLButtonElement>('button[data-action="define"]')!.click();
  await settle();
}

async function snapshot(): Promise<void> {
  host.querySelector<HTMLButtonElement>('button[data-action="snapshot"]')!.click();
  await settle();
}

const hint = () => host.querySelector('[data-members-hint]');

describe('members look', () => {
  it('MB-1 the screen imports its own stylesheet', () => {
    expect(source).toContain("import './members.css';");
  });

  it('MB-2 every section is the shared card', async () => {
    await define();
    await snapshot();
    const sections = [...host.querySelectorAll('section.xc-campaign-detail')];
    expect(sections.length).toBeGreaterThanOrEqual(3);
    for (const s of sections) expect(s.classList.contains('card'), s.textContent ?? '').toBe(true);
  });

  it('MB-3 the save row has room under the fields', () => {
    expect(host.querySelector('.members-screen form')).not.toBeNull();
    const d = decls('.xc-admin .xc-content .members-screen form');
    expect(d?.get('display')).toBe('grid');
    expect(d?.get('gap')).toBe('16px');
  });

  it('MB-4 nothing defined yet: the screen says there is no group and what to do first', () => {
    expect(hint()?.getAttribute('data-members-hint')).toBe('empty');
    expect(hint()?.closest('section.card')).not.toBeNull();
    expect(hint()?.textContent).toContain(th.empty);
    expect(hint()?.textContent).toContain(th.emptyHint);
  });

  it('MB-5 defined, no snapshot yet: the screen says to take a snapshot', async () => {
    await define();
    expect(hint()?.getAttribute('data-members-hint')).toBe('snapshot');
    expect(hint()?.textContent).toBe(th.snapshotHint);
  });

  it('MB-6 a snapshot: no hint, and an empty sample says no member matches', async () => {
    await define();
    await snapshot();
    expect(hint()).toBeNull();
    const cell = host.querySelector('.xc-campaigns-table tbody td[colspan="5"]');
    expect(cell, 'the empty sample row').not.toBeNull();
    expect(cell?.textContent).toBe(th.sampleEmpty);
  });

  it('MB-7 the new texts exist in both languages', () => {
    expect(th.emptyHint).toBe('บันทึกเงื่อนไขกลุ่มก่อน แล้วถ่ายสแนปช็อตเพื่อดูสมาชิกที่ตรงเงื่อนไข');
    expect(th.snapshotHint).toBe('กดถ่ายสแนปช็อตเพื่อดูสมาชิกที่ตรงเงื่อนไข');
    expect(th.sampleEmpty).toBe('ไม่มีสมาชิกที่ตรงเงื่อนไข');
    expect(en.emptyHint).toBe('Save the group conditions first, then take a snapshot to see the matching members');
    expect(en.snapshotHint).toBe('Take a snapshot to see the matching members');
    expect(en.sampleEmpty).toBe('No member matches');
  });

  it('MB-8 the hint is muted 14px text', () => {
    const d = decls('.members-hint');
    expect(d?.get('color')).toBe('var(--xc-muted)');
    expect(d?.get('font-size')).toBe('14px');
  });
});
