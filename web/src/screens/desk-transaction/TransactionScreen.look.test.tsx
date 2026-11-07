// OL-6 wave B dtx-not-found (audit #564 issuecomment-6702970101, shot 009): not-found and load-failed are bare grey text
// under a title that repeats the shell's. Written before the fix by the ticket author claude/studio-claude-17 for the
// sub-controller claude/1cc51615. Do not edit, delete or weaken.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../shared/i18n';
import { makeMe, makeTransactionAuditViewData, makeTransactionViewData } from '../../shared/testing/fixtures';
import { th } from './i18n';
import { TransactionScreen } from './TransactionScreen';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

const ID = '7f0c2b1e-4a5d-4c3b-9e8f-0a1b2c3d4e5f';

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

function stub(transaction: 'null' | 'fails') {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) => {
      const path = new URL(String(input), 'http://xc.test').pathname;
      const view = (data: unknown) => json(200, { version: 1, changed: true, data });
      if (path === '/api/v3/views/me') return view(makeMe({ permissions: ['TransactionRead'] }));
      if (path === '/api/v3/views/transaction') {
        return transaction === 'null'
          ? view(makeTransactionViewData({ transaction: null }))
          : json(500, { error: { code: 'internal', message: 'boom' } });
      }
      if (path === '/api/v3/views/transaction_audit') return view(makeTransactionAuditViewData({}));
      return json(401, { error: { code: 'employeeAuthRequired', message: 'x' } });
    }),
  );
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

async function mount(transaction: 'null' | 'fails') {
  stub(transaction);
  dispose = render(() => <TransactionScreen pathname={`/desk/transactions/${ID}`} search="" />, host);
  for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));
}

let css = '';
let source = '';
let entry = '';

function decls(selector: string): Map<string, string> | undefined {
  const re = /([^{}]+)\{([^{}]*)\}/g;
  const stripped = css.replace(/\/\*[\s\S]*?\*\//g, '');
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
  css = fs.readFileSync(here + 'desk-transaction.css', 'utf8');
  source = fs.readFileSync(here + 'TransactionScreen.tsx', 'utf8');
  entry = fs.readFileSync(here + 'main.tsx', 'utf8');
});

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

describe('OL-6 B transaction-detail empty states', () => {
  it('DT-1 not found: the line sits in a section card marked xc-dtx-empty', async () => {
    await mount('null');
    expect(host.querySelector('[data-state]')!.getAttribute('data-state')).toBe('not-found');
    const empty = host.querySelector('.xc-dtx-section.xc-dtx-empty');
    expect(empty, '.xc-dtx-section.xc-dtx-empty').not.toBeNull();
    expect(empty!.textContent).toContain(th.notFound);
  });

  it('DT-2 load failed: the line and the retry button sit in that card', async () => {
    await mount('fails');
    expect(host.querySelector('[data-state]')!.getAttribute('data-state')).toBe('error');
    const empty = host.querySelector('.xc-dtx-section.xc-dtx-empty');
    expect(empty, '.xc-dtx-section.xc-dtx-empty').not.toBeNull();
    expect(empty!.textContent).toContain(th.loadFailed);
    expect(empty!.querySelector('button[data-action="retry"]')).not.toBeNull();
  });

  it('DT-3 the screen draws no heading of its own; the shell keeps the title', async () => {
    await mount('null');
    expect(host.querySelector('h1')).toBeNull();
    expect(source).not.toContain('xc-dtx-title');
    // Restated for OL-6 B queue-card-f52 by claude/studio-claude-17 (sub-controller claude/1cc51615): the entry now also
    // passes queueCard="hidden" (owner finding F52), so the title is matched as the first attribute, not as the whole tag.
    expect(entry).toMatch(/<AppShell title=\{t\('title'\)\}[\s>]/);
    expect(decls('.xc-dtx-title'), 'no rule left for the removed class').toBeUndefined();
  });

  it('DT-4 the refresh button stays, at the end of the bar', async () => {
    await mount('null');
    expect(host.querySelector('.xc-dtx-bar button[data-action="refresh"]')).not.toBeNull();
    expect(decls('.xc-dtx-bar')?.get('justify-content')).toBe('flex-end');
  });

  it('DT-5 the empty card centres its content in a column', () => {
    const d = decls('.xc-dtx-empty');
    expect(d?.get('display')).toBe('flex');
    expect(d?.get('flex-direction')).toBe('column');
    expect(d?.get('align-items')).toBe('center');
    expect(d?.get('gap')).toBe('12px');
    expect(d?.get('text-align')).toBe('center');
    expect(d?.has('background'), 'the card look stays on .xc-dtx-section').toBe(false);
  });
});
