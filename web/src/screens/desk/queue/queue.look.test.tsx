// OL-6 wave B desk-queue (audit #564 issuecomment-6702970101, shot 005): /desk/queue never loads desk.css and has no
// stylesheet of its own, so the `.desk-*` classes it uses style nothing: sections are a bare h3 + p and the action
// buttons touch. Written before the fix by claude/bbe9ce8b. Do not edit, delete or weaken.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import {
  makeQueueBoardData,
  makeQueueBoardWaiting,
} from '../../../shared/testing/fixtures';
import { Board } from './board/Board';
import { CounterBar } from './counter/CounterBar';

vi.mock('./actions/TicketActions', () => ({
  TicketActions: () => <div class="desk-actions" data-testid="ticket-actions" />,
}));

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
  const stripped = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(stripped)) !== null) {
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

const ME = { userId: 'u-me', permissions: ['QueueOperate'] };

function store(data: unknown): any {
  const [state] = createSignal<any>(data);
  return { state, error: () => undefined, lastUpdated: () => undefined, version: () => undefined, now: () => Date.now(), refresh: vi.fn(async () => {}) };
}

function commands(): any {
  return { issueWalkIn: vi.fn(), callNext: vi.fn(), ticket: vi.fn(), cancel: vi.fn(), counter: vi.fn(), queue: vi.fn() };
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function mount(el: () => JSX.Element): void {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(el, host);
}

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  setLang('th');
});

describe('desk-queue look', () => {
  it('DQ-1 the screen loads its own stylesheet and the shared card, and its root is .desk-queue', async () => {
    const src = await readHere('QueueScreen.tsx');
    expect(src).toMatch(/import '\.\/queue\.css';/);
    expect(src).toMatch(/import '\.\.\/\.\.\/\.\.\/shared\/ui\/card\.css';/);
    expect(src).toMatch(/<div class="desk-queue">/);
  });

  it('DQ-2 .desk-queue stacks with a 16px gap and the desk page gutter', async () => {
    const d = decls(await readHere('queue.css'), '.desk-queue');
    expect(d?.get('display')).toBe('grid');
    expect(d?.get('gap')).toBe('16px');
    expect(d?.get('padding')).toBe('16px');
  });

  it('DQ-3 both board sections are shared cards, also when empty, and the empty line stays', () => {
    mount(() => <Board store={store(makeQueueBoardData({ waiting: [], called: [] }))} me={ME} commands={commands()} />);
    const sections = host.querySelectorAll('section.card.desk-queue-section');
    expect(sections.length).toBe(2);
    expect(host.querySelectorAll('section.desk-queue-section > .desk-empty').length).toBe(2);
  });

  it('DQ-4 an empty line is centred and muted', async () => {
    const d = decls(await readHere('queue.css'), '.desk-queue .desk-empty');
    expect(d?.get('text-align')).toBe('center');
    expect(d?.get('color')).toBe('var(--xc-muted)');
    expect(d?.get('margin')).toBe('0');
  });

  it('DQ-5 tickets are rows in an unmarked list', async () => {
    const data = makeQueueBoardData({
      waiting: [makeQueueBoardWaiting({ id: 'w-1', displayNumber: 'A001' })],
      called: [],
    });
    mount(() => <Board store={store(data)} me={ME} commands={commands()} />);
    expect(host.querySelector('ul.desk-queue-list > li.desk-queue-row[data-ticket="w-1"]')).not.toBeNull();
    const css = await readHere('queue.css');
    const list = decls(css, '.desk-queue-list');
    expect(list?.get('list-style')).toBe('none');
    expect(list?.get('padding')).toBe('0');
    const row = decls(css, '.desk-queue-row');
    expect(row?.get('display')).toBe('flex');
    expect(row?.get('flex-wrap')).toBe('wrap');
  });

  it('DQ-6 the counter bar is a card; only its buttons sit in .desk-actions, the note sits below', () => {
    mount(() => <CounterBar store={store({ waiting: [], called: [], myCounter: null })} me={ME} commands={commands()} />);
    const card = host.querySelector('.card.desk-queue-counter');
    expect(card).not.toBeNull();
    const actions = card!.querySelector(':scope > .desk-actions');
    expect(actions?.querySelector('button[name=issueWalkIn]')).not.toBeNull();
    expect(actions?.querySelector('.desk-notice')).toBeNull();
    expect(card!.querySelector(':scope > .desk-notice')).not.toBeNull();
  });

  it('DQ-7 action buttons have an 8px gap and wrap', async () => {
    const d = decls(await readHere('queue.css'), '.desk-queue .desk-actions');
    expect(d?.get('display')).toBe('flex');
    expect(d?.get('flex-wrap')).toBe('wrap');
    expect(d?.get('gap')).toBe('8px');
  });

  it('DQ-8 call next is the one primary action', () => {
    mount(() => <CounterBar store={store({ waiting: [], called: [], myCounter: { status: 'ACTIVE' } })} me={ME} commands={commands()} />);
    expect(host.querySelector('button[name=callNext]')?.classList.contains('xc-btn--primary')).toBe(true);
    expect(host.querySelectorAll('.xc-btn--primary').length).toBe(1);
  });

  it('DQ-9 the status note is muted, not the desk amber', async () => {
    const d = decls(await readHere('queue.css'), '.desk-queue .desk-notice');
    expect(d?.get('color')).toBe('var(--xc-muted)');
    expect(d?.get('margin')).toBe('0');
  });
});
