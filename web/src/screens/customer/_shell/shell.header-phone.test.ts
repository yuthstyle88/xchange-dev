// OL-1R-e (#6501): customer phone header height regression after OL-1.
// Pins @media (max-width: 760px) overrides under .xc-customer in
// customer/_shell/shell.css back to their 7f820b16 values.
// Parsed the way web/src/shared/ui/QueueCard.float.test.tsx does.
import { describe, expect, it } from 'vitest';

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
  const text = await readSibling('shell.css');
  expect(text, 'web/src/screens/customer/_shell/shell.css exists').toBeDefined();
  return parseCss(text!);
}

const PHONE_MEDIA = '(max-width: 760px)';

describe('customer/_shell/shell.css phone header overrides (OL-1R-e #6501)', () => {
  it('CS-H1: restores .xc-shell-header phone layout declarations to 7f820b16 values', async () => {
    const rules = await sheet();
    const sel = '.xc-customer .xc-shell-header';

    // 7f820b16:web/src/screens/customer/_shell/shell.css:19 display: flex default nowrap
    // 7f820b16:web/src/shared/ui/shell.css:22 display: flex
    expect(value(rules, sel, 'flex-wrap', PHONE_MEDIA)).toBe('nowrap');

    // 7f820b16:web/src/screens/customer/_shell/shell.css:23
    expect(value(rules, sel, 'padding', PHONE_MEDIA)).toBe('8px 16px');

    // 7f820b16:web/src/screens/customer/_shell/shell.css:22
    expect(value(rules, sel, 'gap', PHONE_MEDIA)).toBe('10px');

    // 7f820b16:web/src/screens/customer/_shell/shell.css:21
    expect(value(rules, sel, 'align-items', PHONE_MEDIA)).toBe('center');
  });

  it('CS-H2: restores .xc-shell-title phone declarations to 7f820b16 values', async () => {
    const rules = await sheet();
    const sel = '.xc-customer .xc-shell-header .xc-shell-title';

    // 7f820b16 initial order (default 0); shared shell.css OL-1 set order: 1
    expect(value(rules, sel, 'order', PHONE_MEDIA)).toBe('0');

    // 7f820b16:web/src/screens/customer/_shell/shell.css:57
    expect(value(rules, sel, 'font-size', PHONE_MEDIA)).toBe('12px');

    // 7f820b16 initial overflow (default visible); shared shell.css OL-1 set overflow: hidden
    expect(value(rules, sel, 'overflow', PHONE_MEDIA)).toBe('visible');

    // 7f820b16 initial text-overflow (default clip); shared shell.css OL-1 set text-overflow: ellipsis
    expect(value(rules, sel, 'text-overflow', PHONE_MEDIA)).toBe('clip');

    // 7f820b16 initial white-space (default normal); shared shell.css OL-1 set white-space: nowrap
    expect(value(rules, sel, 'white-space', PHONE_MEDIA)).toBe('normal');
  });

  it('CS-H3: restores .xc-lang-switch phone declarations to 7f820b16 values', async () => {
    const rules = await sheet();
    const sel = '.xc-customer .xc-shell-header > .xc-lang-switch';

    // 7f820b16 initial order (default 0); shared shell.css OL-1 set order: 2
    expect(value(rules, sel, 'order', PHONE_MEDIA)).toBe('0');

    // 7f820b16 initial margin-left; shared shell.css OL-1 set margin-left: auto
    expect(value(rules, sel, 'margin-left', PHONE_MEDIA)).toBe('0');
  });

  it('CS-H4: restores .xc-shell-header > button phone declarations to 7f820b16 values', async () => {
    const rules = await sheet();
    const sel = '.xc-customer .xc-shell-header > button';

    // 7f820b16:web/src/screens/customer/_shell/shell.css:64-76 initial order (default 0)
    expect(value(rules, sel, 'order', PHONE_MEDIA)).toBe('0');
  });
});
