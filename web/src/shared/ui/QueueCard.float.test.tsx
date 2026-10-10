// #786 (web design §12.18.12, QF5–QF7): the shared queue card is the one floating queue notice, on
// admin too, styled once in `web/src/shared/ui/queue-card.css` from v2's `account-bar.js`
// `.xc-queue*` rules plus `admin-workspace.css`'s `.xc-workspace .xc-queue*` overrides
// (xchange-v2-source@4d0bdbb). Written before the implementation by the ticket's author
// (claude/b060e9). Do not edit, delete or weaken; add tests in a file of your own.
//
// QueueCard.test.tsx already pins the fold (−/+, aria-expanded, the phone start, a browser with no
// matchMedia), the badge (— / count / !), class `waiting`, and the unfold on a new waiting line. This
// file pins only what it lacks:
//   - [required QF7] the card's `aside` carries class `queue-card`, and class `minimized` exactly while
//     it is folded (v2 sizes the folded card by that class: `.xc-workspace .xc-queue.minimized`);
//   - [required QF5] `QueueCard.tsx` imports `./queue-card.css`, and that file carries the v2 values
//     that place the card: base, `≤700`, and the admin overrides scoped `body:has(.xc-admin)` (QF6),
//     `≤600` included, with no `body` padding rule.
// The CSS is read as text (vitest loads no CSS): selectors are compared with whitespace collapsed (a
// descendant space is kept), media queries with all whitespace removed, declarations by property, the last one in the file winning.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Accessor } from 'solid-js';
import { render } from 'solid-js/web';
import type { Me } from '../api/generated/common';
import { setLang } from '../i18n';
import { queueStore, type QueueError, type QueueStore } from '../queue/queue-store';
import type { KioskNotifications } from '../queue/wire';
import { QueueCard } from './QueueCard';

// ── fake queue store (as QueueCard.test.tsx's) ───────────────────────────────

interface FakeStore extends QueueStore {
  setState(v: KioskNotifications | undefined): void;
  setError(v: QueueError | undefined): void;
}

const fake = vi.hoisted(() => ({ stores: new Map<string, unknown>() }));

vi.mock('../queue/queue-store', async () => {
  const solid = await import('solid-js');
  const [branch, setBranch] = solid.createSignal('B1');
  const store = (b: string): FakeStore => {
    let s = fake.stores.get(b) as FakeStore | undefined;
    if (!s) {
      const [state, setState] = solid.createSignal<KioskNotifications | undefined>(undefined);
      const [error, setError] = solid.createSignal<QueueError | undefined>(undefined);
      const [lastUpdated] = solid.createSignal<number | undefined>(undefined);
      const [version] = solid.createSignal<number | undefined>(undefined);
      s = {
        state,
        error,
        lastUpdated,
        version,
        now: () => Date.now(),
        async refresh() {},
        setState: (v) => setState(() => v),
        setError: (v) => setError(() => v),
      };
      fake.stores.set(b, s);
    }
    return s;
  };
  return {
    queueBranch: branch,
    setQueueBranch: (b: string) => setBranch(b),
    queueStore: (b: string) => store(b),
    useQueueStore: (b: Accessor<string>) => () => store(b()),
  };
});

const NOK: Me = { kind: 'staff', id: 'E7', role: 'FRONT', branch: 'B1', name: 'Nok', username: 'nok' };

function answer(count: number, latest: string | null = count ? `K${count}` : null) {
  const s = queueStore('B1') as FakeStore;
  s.setError(undefined);
  s.setState({
    serverNow: '2026-10-10T03:00:00.000Z',
    branchId: 'B1',
    branchName: 'Silom',
    count,
    latest,
    workingCount: 0,
    working: [],
    employee: { id: NOK.id, role: NOK.role },
  });
}

function viewport(width: number) {
  const mm = (query: string) => {
    const q = query.replace(/\s+/g, '');
    const max = /max-width:(\d+(?:\.\d+)?)px/.exec(q);
    const min = /min-width:(\d+(?:\.\d+)?)px/.exec(q);
    let matches = max !== null || min !== null;
    if (max) matches = matches && width <= Number(max[1]);
    if (min) matches = matches && width >= Number(min[1]);
    return {
      matches,
      media: query,
      onchange: null,
      addEventListener() {},
      removeEventListener() {},
      addListener() {},
      removeListener() {},
      dispatchEvent: () => false,
    };
  };
  Object.defineProperty(window, 'matchMedia', { value: mm, configurable: true, writable: true });
}

const flush = () => new Promise<void>((r) => setTimeout(r, 0));
let disposers: (() => void)[] = [];

async function mountCard() {
  const host = document.createElement('div');
  document.body.append(host);
  const session = { me: () => NOK, signOut: async () => {}, dispose: () => {} };
  disposers.push(render(() => <QueueCard session={session} />, host));
  await flush();
}

function card(): HTMLElement {
  const all = [...document.querySelectorAll<HTMLElement>('aside')].filter(
    (a) => a.getAttribute('aria-label') === 'แจ้งเตือนคิวลูกค้า',
  );
  expect(all, 'one card labelled แจ้งเตือนคิวลูกค้า').toHaveLength(1);
  return all[0]!;
}

function fold(): HTMLButtonElement {
  const b = card().querySelector<HTMLButtonElement>('button[aria-expanded]');
  expect(b, 'the fold button').not.toBeNull();
  return b!;
}

function expectMinimized(want: boolean, why: string) {
  expect(fold().getAttribute('aria-expanded'), `${why}: aria-expanded`).toBe(want ? 'false' : 'true');
  expect(card().classList.contains('minimized'), `${why}: class minimized on the card`).toBe(want);
}

beforeEach(() => {
  setLang('th');
  fake.stores.clear();
  Reflect.deleteProperty(window, 'matchMedia');
  vi.stubGlobal('fetch', vi.fn(async () => new Response('{}', { status: 500 })));
});

afterEach(() => {
  for (const d of disposers.splice(0)) d();
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  Reflect.deleteProperty(window, 'matchMedia');
});

// ── the classes the CSS keys on (QF7) ────────────────────────────────────────

describe('the card classes (QF7)', () => {
  it('CF1: the card is aside.queue-card, unfolded without class minimized', async () => {
    await mountCard();
    expect(card().classList.contains('queue-card')).toBe(true);
    expectMinimized(false, 'at mount, no matchMedia');
  });

  it('CF2: folding by the button adds minimized; unfolding removes it', async () => {
    await mountCard();
    fold().click();
    await flush();
    expectMinimized(true, 'after one click');
    fold().click();
    await flush();
    expectMinimized(false, 'after two clicks');
  });

  it('CF3: a 700px viewport starts minimized; a waiting count unfolds it and drops minimized', async () => {
    viewport(700);
    await mountCard();
    expectMinimized(true, 'phone at mount');
    answer(0);
    await flush();
    expectMinimized(true, 'count 0');
    answer(2);
    await flush();
    expectMinimized(false, 'count 0 → 2');
  });

  it('CF4: class waiting sits on the same aside.queue-card', async () => {
    await mountCard();
    answer(1);
    await flush();
    expect(card().classList.contains('queue-card') && card().classList.contains('waiting')).toBe(true);
  });
});

// ── the stylesheet (QF5, QF6) ────────────────────────────────────────────────

interface NodeFs {
  readFileSync(file: string, encoding: string): string;
  existsSync(file: string): boolean;
}
interface NodePath {
  dirname(p: string): string;
  join(...segments: string[]): string;
}

type Rule = { media: string; selector: string; decls: Map<string, string> };

/** Flat rules of a stylesheet: one entry per selector of each rule, inside at most one @media. */
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

/** The value of `prop` for exactly `selector` under `media` ('' = no media), the last one winning. */
function value(rules: Rule[], selector: string, prop: string, media = ''): string | undefined {
  const sel = selector.trim().replace(/\s+/g, ' ').replace(/\s*([>+~(),])\s*/g, '$1');
  const med = media.replace(/\s+/g, '');
  let v: string | undefined;
  for (const r of rules) if (r.selector === sel && r.media === med && r.decls.has(prop)) v = r.decls.get(prop);
  return v;
}

let fs: NodeFs;
let path: NodePath;
let here: string;

async function readSibling(name: string): Promise<string | undefined> {
  if (!fs) {
    fs = (await import(/* @vite-ignore */ 'node:fs' as string)) as unknown as NodeFs;
    path = (await import(/* @vite-ignore */ 'node:path' as string)) as unknown as NodePath;
    here = path.dirname(new URL(import.meta.url).pathname);
  }
  const file = path.join(here, name);
  return fs.existsSync(file) ? fs.readFileSync(file, 'utf8') : undefined;
}

async function sheet(): Promise<Rule[]> {
  const text = await readSibling('queue-card.css');
  expect(text, 'web/src/shared/ui/queue-card.css exists').toBeDefined();
  return parseCss(text!);
}

function expectDecls(rules: Rule[], selector: string, media: string, want: Record<string, string>) {
  for (const [prop, v] of Object.entries(want)) {
    expect(value(rules, selector, prop, media), `${media || '(no media)'} ${selector} { ${prop} }`).toBe(v);
  }
}

describe('queue-card.css (QF5, QF6)', () => {
  it('CS1: QueueCard.tsx imports ./queue-card.css (look rule 6)', async () => {
    const src = await readSibling('QueueCard.tsx');
    expect(src).toMatch(/import\s+['"]\.\/queue-card\.css['"]/);
  });

  it('CS2: the base card is v2 .xc-queue: fixed bottom-right, 320px, radius 20px (account-bar.js @2212)', async () => {
    const r = await sheet();
    expectDecls(r, '.queue-card', '', {
      position: 'fixed',
      bottom: '20px',
      right: '20px',
      'z-index': '1000',
      width: '320px',
      'max-width': 'calc(100vw - 32px)',
      padding: '18px',
      background: '#fff',
      border: '1px solid #e8dfe4',
      'border-radius': '20px',
      'box-shadow': '0 10px 38px #30202b22',
    });
    expectDecls(r, '.queue-card.waiting', '', { 'border-color': '#ab0044' });
  });

  it('CS3: v2\'s ≤600 and ≤700 card rules (account-bar.js @3685, @4526)', async () => {
    const r = await sheet();
    expectDecls(r, '.queue-card', '(max-width:600px)', { right: '12px', bottom: '12px', width: '290px' });
    expectDecls(r, '.queue-card', '(max-width:700px)', {
      left: '0',
      right: '0',
      bottom: '0',
      width: 'auto',
      'max-width': 'none',
      'border-radius': '16px 16px 0 0',
      padding: '10px 14px',
    });
  });

  it('CS4: the admin overrides, scoped body:has(.xc-admin) (admin-workspace.css:29)', async () => {
    const r = await sheet();
    expectDecls(r, 'body:has(.xc-admin) .queue-card', '', {
      width: '295px',
      'border-radius': '10px',
      'box-shadow': '0 5px 24px #28142114',
      'z-index': '25',
      padding: '14px',
    });
    expectDecls(r, 'body:has(.xc-admin) .queue-card.minimized', '', { width: '280px' });
  });

  it('CS5: the admin ≤600 overrides (admin-workspace.css:34)', async () => {
    const r = await sheet();
    expectDecls(r, 'body:has(.xc-admin) .queue-card', '(max-width:600px)', {
      left: '12px',
      right: '12px',
      bottom: '10px',
      width: 'auto',
      'max-width': 'none',
    });
    expectDecls(r, 'body:has(.xc-admin) .queue-card.minimized', '(max-width:600px)', { left: 'auto', width: '260px' });
  });

  it('CS6: no body padding rule (v2 @4739 body{padding-bottom:84px} is not ported) and nothing on :root', async () => {
    const r = await sheet();
    expect(r.filter((x) => x.selector === 'body' || x.selector === ':root').map((x) => `${x.media} ${x.selector}`)).toEqual([]);
  });
});
