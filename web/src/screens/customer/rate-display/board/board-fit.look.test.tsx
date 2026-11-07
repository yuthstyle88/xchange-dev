// OL-6 wave B rate-board: the board fits its canvas, the font, the toolbar outside the canvas, the body margin. Written
// before the implementation by the test-writer claude/94389a6b on the sub-controller claude/1cc51615's order. Do not
// edit, delete or weaken.
import { render } from 'solid-js/web';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import type { BoardModel } from './board-model';
import { BoardShell, type BoardShellProps } from './BoardShell';

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

const MODEL: BoardModel = { headings: [], singleColumn: true, rows: [] };
const PROPS: BoardShellProps = {
  branch: 'MAIN',
  branchName: 'Main',
  model: MODEL,
  status: { status: 'synced', ratesVisible: true, emptyMessage: null },
  stale: false,
  lastOkAt: undefined,
  sourceDate: null,
  snapshotId: null,
  hours: null,
  bookingEnabled: false,
  attract: false,
};

let dispose: (() => void) | undefined;
let host: HTMLDivElement;

function setWindow(width: number, height: number): void {
  Object.defineProperty(window, 'innerWidth', { writable: true, configurable: true, value: width });
  Object.defineProperty(window, 'innerHeight', { writable: true, configurable: true, value: height });
}

/** The button the way RateDisplay renders it: before the board, with its bottom edge `bottom` px from the window top. */
function addToolbar(bottom: number): HTMLButtonElement {
  const button = document.createElement('button');
  button.className = 'xc-rate-display-fullscreen-btn';
  button.getBoundingClientRect = () => ({ bottom }) as DOMRect;
  document.body.prepend(button);
  return button;
}

function board(): HTMLElement {
  dispose = render(() => <BoardShell {...PROPS} />, host);
  return host.querySelector<HTMLElement>('.xc-rb')!;
}

describe('rate board fit (OL-6 wave B)', () => {
  let shell: Rule[];
  let page: Rule[];
  const rule = (list: Rule[], selector: string) => list.find((r) => r.selector === selector);

  beforeAll(async () => {
    const specifier = 'node:fs';
    const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
    const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
    shell = parseRules(fs.readFileSync(here + 'board-shell.css', 'utf8'));
    page = parseRules(fs.readFileSync(here + '../rate-display.css', 'utf8'));
  });

  beforeEach(() => {
    host = document.createElement('div');
    document.body.append(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    document.body.innerHTML = '';
    vi.restoreAllMocks();
  });

  it('RB-1: everything on the canvas is border-box, so a 50% price cell with padding stays on one line', () => {
    expect(rule(shell, '.xc-rb, .xc-rb *')?.decls.get('box-sizing')).toBe('border-box');
  });

  it('RB-2: only the table gives way when the canvas is short; the header, stub and QR keep their height', () => {
    expect(rule(shell, '.xc-rb > :not(.xc-rb-table)')?.decls.get('flex')).toBe('none');
  });

  it('RB-3: the board font is the theme sans; it was the browser serif', () => {
    expect(rule(shell, '.xc-rb')?.decls.get('font-family')).toBe('var(--xc-font-sans)');
  });

  it('RB-4: the page font no longer inherits the browser serif', () => {
    expect(rule(page, '.xc-rate-display')?.decls.get('font-family')).toBe('var(--xc-font-sans)');
  });

  it('RB-5: the page resets the default body margin', () => {
    expect(rule(page, 'body:has(.xc-rate-display)')?.decls.get('margin')).toBe('0');
  });

  it('RB-6: a page holding the board has no padding of its own; the message states keep theirs', () => {
    expect(rule(page, '.xc-rate-display:has(.xc-rb)')?.decls.get('padding')).toBe('0');
    expect(rule(page, '.xc-rate-display')?.decls.get('padding')).toBe('1.5rem 2rem');
  });

  it('RB-7: the button is its own size at the top right, not a full-width bar', () => {
    const r = rule(page, '.xc-rate-display > .xc-rate-display-fullscreen-btn');
    expect(r?.decls.get('align-self')).toBe('flex-end');
    expect(r?.decls.get('min-height')).toBe('44px');
  });

  it('RB-8: the board is placed by --xc-rb-top, and at the middle without it', () => {
    expect(rule(shell, '.xc-rb')?.decls.get('top')).toBe('var(--xc-rb-top, 50%)');
  });

  it('RB-9: no button (full screen): the scale is the design formula and the board is centred', () => {
    setWindow(1080, 1920);
    const el = board();
    expect(el.style.getPropertyValue('--scale')).toBe('1');
    expect(el.style.getPropertyValue('--xc-rb-top')).toBe('960px');
  });

  it('RB-10: a button: the board scales into the height under it, 8px clear, and centres there', () => {
    addToolbar(52);
    setWindow(1080, 1980);
    const el = board();
    expect(el.style.getPropertyValue('--scale')).toBe('1');
    expect(el.style.getPropertyValue('--xc-rb-top')).toBe('1020px');
  });

  it('RB-11: a button in a narrow window: width still decides the scale', () => {
    addToolbar(52);
    setWindow(540, 1980);
    const el = board();
    expect(el.style.getPropertyValue('--scale')).toBe('0.5');
    expect(el.style.getPropertyValue('--xc-rb-top')).toBe('1020px');
  });

  it('RB-12: entering full screen removes the button: the board takes the whole height back', () => {
    const button = addToolbar(52);
    setWindow(1080, 1980);
    const el = board();
    button.remove();
    document.dispatchEvent(new Event('fullscreenchange'));
    expect(el.style.getPropertyValue('--xc-rb-top')).toBe('990px');
  });

  it('RB-13: unmount removes the fullscreenchange listener', () => {
    const add = vi.spyOn(document, 'addEventListener');
    const remove = vi.spyOn(document, 'removeEventListener');
    setWindow(1080, 1920);
    board();
    const listener = add.mock.calls.find((c) => c[0] === 'fullscreenchange')?.[1];
    expect(listener).toBeDefined();
    dispose?.();
    dispose = undefined;
    expect(remove).toHaveBeenCalledWith('fullscreenchange', listener);
  });
});
