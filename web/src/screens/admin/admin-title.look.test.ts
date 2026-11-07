// OL-6 L4 follow-up (#6543): admin title padding and phone panel full width
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: string): string;
}
interface NodePath {
  dirname(p: string): string;
  join(...segments: string[]): string;
}

type Rule = { atRule: string; selector: string; decls: Map<string, string> };

function parseCss(text: string): Rule[] {
  const raw = text.replace(/\/\*[\s\S]*?\*\//g, '');
  const out: Rule[] = [];
  const normSel = (s: string) => s.trim().replace(/\s+/g, ' ').replace(/\s*([>+~(),])\s*/g, '$1');
  const block = (body: string, atRule: string) => {
    const re = /([^{}]+)\{([^{}]*)\}/g;
    let m: RegExpExecArray | null;
    while ((m = re.exec(body)) !== null) {
      const decls = new Map<string, string>();
      for (const d of m[2]!.split(';')) {
        const k = d.indexOf(':');
        if (k > 0) decls.set(d.slice(0, k).trim().toLowerCase(), d.slice(k + 1).trim().replace(/\s+/g, ' '));
      }
      for (const sel of m[1]!.split(',')) out.push({ atRule, selector: normSel(sel), decls });
    }
  };

  let rest = '';
  let i = 0;
  while (i < raw.length) {
    const at = raw.indexOf('@media', i);
    if (at < 0) {
      rest += raw.slice(i);
      break;
    }
    rest += raw.slice(i, at);
    const open = raw.indexOf('{', at);
    if (open < 0) {
      rest += raw.slice(at);
      break;
    }
    let depth = 1;
    let j = open + 1;
    while (j < raw.length && depth > 0) {
      if (raw[j] === '{') depth += 1;
      if (raw[j] === '}') depth -= 1;
      j += 1;
    }
    const header = raw.slice(at, open).trim();
    block(raw.slice(open + 1, j - 1), header);
    i = j;
  }
  block(rest, '');
  return out;
}

describe('admin title and phone panel look (#6543)', () => {
  let rules: Rule[] = [];

  beforeAll(async () => {
    const fs = (await import(/* @vite-ignore */ 'node:fs' as string)) as unknown as NodeFs;
    const path = (await import(/* @vite-ignore */ 'node:path' as string)) as unknown as NodePath;
    const here = path.dirname(new URL(import.meta.url).pathname);
    rules = parseCss(fs.readFileSync(path.join(here, 'admin.css'), 'utf8'));
  });

  it('admin title above 760 px: left edge matches content (padding-left: 38px, reaching x 286 at 1280)', () => {
    const titleRule = rules.find(
      (r) =>
        r.atRule === '' &&
        r.selector.includes('.xc-shell-title') &&
        r.selector.startsWith('.xc-admin') &&
        r.decls.has('padding-left'),
    );
    expect(titleRule, 'title has padding-left above breakpoint').toBeDefined();
    expect(titleRule?.decls.get('padding-left')).toBe('38px');
  });

  it('admin title at ≤ 760 px: has content side padding (padding-left: 16px)', () => {
    const media = '(max-width: 760px)';
    const titlePhoneRule = rules.find(
      (r) =>
        r.atRule.replace(/\s+/g, '').includes(media.replace(/\s+/g, '')) &&
        r.selector.includes('.xc-shell-title') &&
        r.selector.startsWith('.xc-admin') &&
        r.decls.has('padding-left'),
    );
    expect(titlePhoneRule, 'title has padding-left at <= 760px').toBeDefined();
    expect(titlePhoneRule?.decls.get('padding-left')).toBe('16px');
  });

  it('admin phone panel: spans full viewport width (calc(100% + 36px) and margin 0 -18px)', () => {
    const media = '(max-width: 760px)';
    const panelRule = rules.find(
      (r) =>
        r.atRule.replace(/\s+/g, '').includes(media.replace(/\s+/g, '')) &&
        r.selector === '.xc-admin .xc-shell-panel',
    );
    expect(panelRule, '.xc-admin .xc-shell-panel rule exists at <= 760px').toBeDefined();
    expect(panelRule?.decls.get('width')).toBe('calc(100% + 36px)');
    const margin = panelRule?.decls.get('margin') || panelRule?.decls.get('margin-inline');
    expect(margin).toMatch(/-18px/);
  });
});
