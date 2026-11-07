// OL-6 wave B queue-display: phone layout, empty panels, body margin, heading contrast. Written before the
// implementation by the test-writer claude/94389a6b on the sub-controller claude/1cc51615's order. Do not edit, delete
// or weaken.
import { render } from 'solid-js/web';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { setAudience, setLang } from '../../../shared/i18n';
import { QueueDisplay } from './QueueDisplay';
import { th } from './i18n';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

type Rule = { selector: string; decls: Map<string, string> };

function parseRules(css: string): Rule[] {
  const stripped = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const rules: Rule[] = [];
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(stripped)) !== null) {
    const decls = new Map<string, string>();
    for (const d of m[2]!.split(';')) {
      const idx = d.indexOf(':');
      if (idx > 0) decls.set(d.slice(0, idx).trim().toLowerCase(), d.slice(idx + 1).trim());
    }
    rules.push({ selector: m[1]!.trim().replace(/\s+/g, ' '), decls });
  }
  return rules;
}

/** The body of the `@media` block whose condition is exactly `condition`. */
function mediaBody(css: string, condition: string): string {
  const at = css.indexOf(`@media ${condition}`);
  if (at < 0) return '';
  const open = css.indexOf('{', at);
  let depth = 1;
  let i = open + 1;
  while (i < css.length && depth > 0) {
    if (css[i] === '{') depth += 1;
    if (css[i] === '}') depth -= 1;
    i += 1;
  }
  return css.slice(open + 1, i - 1);
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let answer: unknown;

const fetchFn = vi.fn(async (url: string) => {
  if (url === '/api/public/host') return json(200, { org: 'ORG_1' });
  return json(200, answer);
});

async function settle(): Promise<void> {
  await vi.advanceTimersByTimeAsync(0);
  for (let i = 0; i < 5; i += 1) await Promise.resolve();
}

function mount(): void {
  dispose = render(() => <QueueDisplay fetch={fetchFn} search="?branch=MAIN" intervalMs={5_000} />, host);
}

const sections = () => [...host.querySelectorAll<HTMLElement>('.queue-display-section')];

describe('queue-display look (OL-6 wave B)', () => {
  let css: string;
  let rules: Rule[];
  const rule = (selector: string) => rules.find((r) => r.selector === selector);

  beforeAll(async () => {
    const specifier = 'node:fs';
    const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
    const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
    css = fs.readFileSync(here + 'queue-display.css', 'utf8');
    rules = parseRules(css);
  });

  beforeEach(() => {
    vi.useFakeTimers();
    setAudience('customer');
    setLang('th');
    answer = { version: 1, changed: true, data: { called: [], recent: [] } };
    host = document.createElement('div');
    document.body.append(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.useRealTimers();
  });

  it('QD-1: nothing recent: the recent panel shows the empty text', async () => {
    mount();
    await settle();
    expect(sections()[1]?.querySelector('.queue-display-empty')?.textContent).toBe(th.empty);
  });

  it('QD-2: a recent number: the recent panel has no empty text', async () => {
    answer = { version: 1, changed: true, data: { called: [], recent: [{ displayNumber: 'K041' }] } };
    mount();
    await settle();
    expect(sections()[1]?.querySelector('[data-recent="K041"]')).not.toBeNull();
    expect(sections()[1]?.querySelector('.queue-display-empty')).toBeNull();
  });

  it('QD-3: the page resets the default body margin', () => {
    expect(rule('body:has(.queue-display-screen)')?.decls.get('margin')).toBe('0');
  });

  it('QD-4: headings use the ink colour, not muted', () => {
    expect(rule('.queue-display-heading')?.decls.get('color')).toBe('var(--xc-ink)');
  });

  it('QD-5: at 760px and below the grid is one column and does not stretch', () => {
    const phone = parseRules(mediaBody(css, '(max-width: 760px)'));
    const grid = phone.find((r) => r.selector === '.queue-display-grid');
    expect(grid?.decls.get('grid-template-columns')).toBe('1fr');
    expect(grid?.decls.get('flex')).toBe('none');
  });

  it('QD-6: at 760px and below the page gutter is 16px', () => {
    const phone = parseRules(mediaBody(css, '(max-width: 760px)'));
    expect(phone.find((r) => r.selector === '.queue-display-screen')?.decls.get('padding')).toBe('16px');
  });

  it('QD-7: the two-column grid stays above the breakpoint', () => {
    expect(rule('.queue-display-grid')?.decls.get('grid-template-columns')).toBe('2fr 1fr');
  });
});
