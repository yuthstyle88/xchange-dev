// #4302: written before the fix by the ticket author claude/studio-claude-4 (sub-controller under XC FE claude/e92db6),
// on the FE controller's ruling A (#4302 issuecomment-6321340194). Do not edit, delete or weaken; add tests in a file of
// your own.
//
// The floating queue card (`web/src/shared/ui/queue-card.css`, fixed bottom-right) covered admin controls, e.g. the
// approvals `อนุมัติ` button. Ruling A: while the card shows, the admin content container reserves room for it: a right
// gutter at ≥1024 px, a bottom gutter below that. Never on `body`. QueueCard.float.test.tsx (QF5–QF7) stays as written,
// so the card's own values stay literals there, and this file ties the gutter to them instead.
//
// jsdom lays nothing out, so a "control outside the card's box" check cannot run here. These tests pin the CSS that
// makes the room, read as text (like QueueCard.float.test.tsx); the end-to-end proof is qa/e2e F12b leg 2 on a host.
//   - [required] G1: ≥1024 px: `.xc-admin:has(.queue-card) .xc-content .admin-main` has `padding-right` of at least the
//     admin card's width + its right offset, both read from queue-card.css;
//   - [required] G2: ≤1023 px: the same selector has `padding-bottom` of at least 240px (an open admin card's height);
//   - [required] G3: the gutter is scoped to "while the card shows" (`:has(.queue-card)`), so a page with no card keeps
//     today's padding;
//   - [required] G4: no rule in admin.css or queue-card.css that targets `body` itself (`body`, `body:has(…)`, no
//     descendant) declares any padding.
import { describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: string): string;
  existsSync(file: string): boolean;
}
interface NodePath {
  join(...segments: string[]): string;
  dirname(p: string): string;
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

function value(rules: Rule[], selector: string, prop: string, media = ''): string | undefined {
  const sel = selector.trim().replace(/\s+/g, ' ').replace(/\s*([>+~(),])\s*/g, '$1');
  const med = media.replace(/\s+/g, '');
  let v: string | undefined;
  for (const r of rules) if (r.selector === sel && r.media === med && r.decls.has(prop)) v = r.decls.get(prop);
  return v;
}

function px(v: string | undefined, what: string): number {
  const m = /^(\d+(?:\.\d+)?)px$/.exec(v ?? '');
  expect(m, `${what} is a px length (got ${v})`).not.toBeNull();
  return Number(m![1]);
}

async function read(rel: string): Promise<Rule[]> {
  const fs = (await import(/* @vite-ignore */ 'node:fs' as string)) as unknown as NodeFs;
  const path = (await import(/* @vite-ignore */ 'node:path' as string)) as unknown as NodePath;
  const file = path.join(path.dirname(new URL(import.meta.url).pathname), rel);
  expect(fs.existsSync(file), `${rel} exists`).toBe(true);
  return parseCss(fs.readFileSync(file, 'utf8'));
}

const GUTTERED = '.xc-admin:has(.queue-card) .xc-content .admin-main';

describe('#4302 the admin content container makes room for the queue card', () => {
  it('[required] G1: ≥1024px, padding-right ≥ the admin card width + its right offset', async () => {
    const card = await read('../../shared/ui/queue-card.css');
    const admin = await read('admin.css');
    const width = px(value(card, 'body:has(.xc-admin) .queue-card', 'width'), 'admin card width');
    const right = px(value(card, '.queue-card', 'right'), 'card right offset');
    const gutter = px(value(admin, GUTTERED, 'padding-right', '(min-width:1024px)'), `${GUTTERED} padding-right ≥1024`);
    expect(gutter).toBeGreaterThanOrEqual(width + right);
  });

  it('[required] G2: ≤1023px, padding-bottom ≥ 240px', async () => {
    const admin = await read('admin.css');
    const gutter = px(value(admin, GUTTERED, 'padding-bottom', '(max-width:1023px)'), `${GUTTERED} padding-bottom ≤1023`);
    expect(gutter).toBeGreaterThanOrEqual(240);
  });

  it('[required] G3: the gutter applies only while the card shows', async () => {
    const admin = await read('admin.css');
    const gutterRules = admin.filter(
      (r) => r.selector.includes('.admin-main') && (r.decls.has('padding-right') || r.decls.has('padding-bottom')) &&
        (r.media === '(min-width:1024px)' || r.media === '(max-width:1023px)'),
    );
    expect(gutterRules.length, 'a gutter rule exists').toBeGreaterThan(0);
    for (const r of gutterRules) expect(r.selector, `${r.media} ${r.selector}`).toContain(':has(.queue-card)');
  });

  it('[required] G4: no rule on body itself declares padding in admin.css or queue-card.css', async () => {
    for (const rel of ['admin.css', '../../shared/ui/queue-card.css']) {
      const rules = await read(rel);
      const bad = rules
        .filter((r) => /^body(:has\([^()]*\))?$/.test(r.selector) && [...r.decls.keys()].some((k) => k.startsWith('padding')))
        .map((r) => `${rel} ${r.media} ${r.selector}`);
      expect(bad).toEqual([]);
    }
  });
});
