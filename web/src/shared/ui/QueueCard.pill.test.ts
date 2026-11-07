// OL-7 queue-card-pill (sub-controller claude/1cc51615's decision, /tmp/ol7/ORDER-claude-19-b.md item 1, option A, ~22:10
// ICT 2026-11-07). The floating queue card covered page content inside the window before any scroll: on admin pages
// (verifier's note outside the 138 rows, e.g. the slips status filter and the loyalty draft form), on /staff/report (row 27)
// and on the staff guide (row 30). Measured by claude/94389a6b on xc-v3 build 45e8457: the bottom reserve
// (--xc-queue-card-space, QueueCard.space.test.tsx) already clears every page's end, so what is left is the card's own size
// while nothing waits. Decided:
//   - folded = a pill: badge + toggle only, no wider than 140px, in the bottom-right corner, on every page and width (at
//     1280px the content column ends at x 1120, leaving 160px beside it);
//   - the folded card's status line stays in the DOM as a live region (role="status", aria-live), only visually hidden, so a
//     screen reader still hears "มีคิวลูกค้ารอ …";
//   - admin pages and the staff guide start folded (`<AppShell queueCard="folded">`, as /staff/report already does,
//     staff-report/main.tsx:16). The card still unfolds by itself when a customer waits (QueueCard.test.tsx "count 0 → 2
//     while folded … and it unfolds").
// Written before the implementation by claude/94389a6b. Do not edit, delete or weaken; add tests in a file of your own.
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

interface Rule {
  media: string;
  selector: string;
  decls: Map<string, string>;
}

let rules: Rule[] = [];
let read: (rel: string) => string = () => '';

/** Rules of a stylesheet, one per selector, with the @media text (whitespace removed) they sit in. */
function parse(css: string): Rule[] {
  const out: Rule[] = [];
  const src = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const push = (media: string, sel: string, body: string) => {
    const decls = new Map<string, string>();
    for (const d of body.split(';')) {
      const i = d.indexOf(':');
      if (i > 0) decls.set(d.slice(0, i).trim().toLowerCase(), d.slice(i + 1).trim().replace(/\s+/g, ' '));
    }
    for (const s of sel.split(',')) out.push({ media, selector: s.trim().replace(/\s+/g, ' '), decls });
  };
  let i = 0;
  while (i < src.length) {
    const open = src.indexOf('{', i);
    if (open < 0) break;
    const head = src.slice(i, open).trim();
    if (head.startsWith('@media')) {
      let depth = 1;
      let j = open + 1;
      while (j < src.length && depth > 0) {
        if (src[j] === '{') depth += 1;
        if (src[j] === '}') depth -= 1;
        j += 1;
      }
      const inner = src.slice(open + 1, j - 1);
      for (const m of inner.matchAll(/([^{}]+)\{([^{}]*)\}/g)) push(head.replace(/^@media/, '').replace(/\s+/g, ''), m[1]!, m[2]!);
      i = j;
    } else {
      const close = src.indexOf('}', open);
      push('', head, src.slice(open + 1, close));
      i = close + 1;
    }
  }
  return out;
}

/** The value of `prop` for `selector` in `media` ('' = no media), the last rule in the file winning. */
function value(selector: string, prop: string, media = ''): string | undefined {
  let v: string | undefined;
  for (const r of rules) if (r.selector === selector && r.media === media && r.decls.has(prop)) v = r.decls.get(prop);
  return v;
}

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  read = (rel) => fs.readFileSync(here + rel, 'utf8');
  rules = parse(read('queue-card.css'));
});

describe('folded queue card is a pill', () => {
  it('QP-P1 folded, on every page: no fixed width, at most 140px, right-anchored (left auto)', () => {
    expect(value('.queue-card.minimized', 'width')).toBe('auto');
    expect(value('.queue-card.minimized', 'max-width')).toBe('140px');
    expect(value('.queue-card.minimized', 'left')).toBe('auto');
  });

  it('QP-P2 folded on a phone: no ≤ 700px or ≤ 600px rule widens the folded card again', () => {
    // QP-P1's `.queue-card.minimized` (two classes) already beats the phone `.queue-card` rules (one class); this guards
    // against a later phone rule for the folded card that sets a width or a left edge.
    for (const media of ['(max-width:700px)', '(max-width:600px)']) {
      expect(value('.queue-card.minimized', 'width', media) ?? 'auto').toBe('auto');
      expect(value('.queue-card.minimized', 'left', media) ?? 'auto').toBe('auto');
    }
  });

  it('QP-P3 folded on admin: the 295px admin width does not apply (auto, at most 140px)', () => {
    expect(value('body:has(.xc-admin) .queue-card.minimized', 'width')).toBe('auto');
    expect(value('body:has(.xc-admin) .queue-card.minimized', 'max-width')).toBe('140px');
  });

  it('QP-P4 folded: the title is not drawn', () => {
    expect(value('.queue-card.minimized .queue-card-title', 'display')).toBe('none');
  });

  it('QP-P5 folded: the status line is visually hidden but stays a live region (no display:none)', () => {
    const sel = '.queue-card.minimized .queue-card-status';
    expect(value(sel, 'display')).toBeUndefined();
    expect(value(sel, 'visibility')).toBeUndefined();
    expect(value(sel, 'position')).toBe('absolute');
    expect(value(sel, 'width')).toBe('1px');
    expect(value(sel, 'height')).toBe('1px');
    expect(value(sel, 'overflow')).toBe('hidden');
    expect(value(sel, 'clip-path')).toBe('inset(50%)');
    expect(value(sel, 'white-space')).toBe('nowrap');
  });

  it('QP-P6 unchanged: the open card keeps its size (320px; admin 295px)', () => {
    expect(value('.queue-card', 'width')).toBe('320px');
    expect(value('body:has(.xc-admin) .queue-card', 'width')).toBe('295px');
  });
});

describe('pages that start with the card folded', () => {
  /** The opening tag of the first <AppShell …> element in a source file. */
  function appShellTag(src: string): string {
    const at = src.indexOf('<AppShell');
    expect(at, '<AppShell present').toBeGreaterThanOrEqual(0);
    let depth = 0;
    for (let i = at; i < src.length; i++) {
      if (src[i] === '{') depth += 1;
      if (src[i] === '}') depth -= 1;
      if (src[i] === '>' && depth === 0 && src[i - 1] !== '=') return src.slice(at, i + 1);
    }
    return src.slice(at);
  }

  it('QP-P7 admin pages: AdminShell passes queueCard="folded"', () => {
    expect(appShellTag(read('../../screens/admin/AdminShell.tsx'))).toContain('queueCard="folded"');
  });

  it('QP-P8 the staff guide passes queueCard="folded" (row 30)', () => {
    expect(appShellTag(read('../../screens/staff-guide/StaffGuide.tsx'))).toContain('queueCard="folded"');
  });

  it('QP-P9 unchanged: /staff/report already passes queueCard="folded" (row 27)', () => {
    expect(appShellTag(read('../../screens/staff-report/main.tsx'))).toContain('queueCard="folded"');
  });
});
