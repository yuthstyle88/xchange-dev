// OL-6 wave B admin (audit #564 issuecomment-6702970101, row 037): the campaigns list's look. One "สร้างแคมเปญ" at a
// time (the empty state's when there is nothing yet, the header's otherwise), the empty state is the shared white card
// instead of a dashed grey box, and the list no longer repeats the page title. Written before the fix by
// claude/94389a6b. Do not edit, delete or weaken; add your own elsewhere.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { makeCampaignListItem, makeCampaignsData } from '../../../shared/testing/fixtures';
import { Campaigns } from './Campaigns';
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
  css = fs.readFileSync(here + 'campaigns.css', 'utf8');
});

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

async function mount(campaigns: ReturnType<typeof makeCampaignListItem>[]): Promise<void> {
  vi.stubGlobal(
    'fetch',
    vi.fn(
      async () =>
        new Response(JSON.stringify({ version: 1, changed: true, data: makeCampaignsData({ campaigns }) }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        }),
    ),
  );
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-11-07', to: '2026-11-07' });
  const [shown] = createSignal(true);
  dispose = render(() => <Campaigns branch={branch} period={period} shown={shown} />, host);
  for (let i = 0; i < 8; i++) await new Promise((r) => setTimeout(r, 0));
}

beforeEach(() => {
  setLang('th');
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.unstubAllGlobals();
});

const creates = () => [...host.querySelectorAll('button')].filter((b) => b.textContent?.trim() === th.create);

describe('campaigns list look', () => {
  it('CA-1 no campaigns: one create button, the empty state\'s', async () => {
    await mount([]);
    expect(host.querySelector('.xc-campaigns-empty'), 'the empty state').not.toBeNull();
    expect(creates()).toHaveLength(1);
    expect(creates()[0]!.closest('.xc-campaigns-empty')).not.toBeNull();
  });

  it('CA-2 some campaigns: one create button, the header\'s', async () => {
    await mount([makeCampaignListItem({ id: 'cmp-1', name: 'Golden October' })]);
    expect(host.querySelector('.xc-campaigns-table'), 'the list').not.toBeNull();
    expect(creates()).toHaveLength(1);
    expect(creates()[0]!.closest('.xc-campaigns-header')).not.toBeNull();
  });

  it('CA-3 the empty state is the shared card, with no dashed grey copy of its own', async () => {
    await mount([]);
    expect(host.querySelector('.xc-campaigns-empty')!.classList.contains('card')).toBe(true);
    const d = decls('.xc-campaigns-empty');
    for (const p of ['border', 'background-color', 'border-radius']) expect(d?.has(p), p).toBe(false);
  });

  it('CA-4 the list does not repeat the page title (the shell heading shows it)', async () => {
    await mount([makeCampaignListItem({ id: 'cmp-1', name: 'Golden October' })]);
    const headings = [...host.querySelectorAll('h1, h2, h3')].map((h) => h.textContent?.trim());
    expect(headings).not.toContain(th.title);
  });

  it('CA-5 the header keeps its button at the right edge', () => {
    expect(decls('.xc-campaigns-header')?.get('justify-content')).toBe('flex-end');
  });
});
