import { beforeAll, describe, expect, it } from 'vitest';

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
    const selector = m[1]!.trim();
    const decls = new Map<string, string>();
    for (const d of m[2]!.split(';')) {
      const idx = d.indexOf(':');
      if (idx > 0) {
        decls.set(d.slice(0, idx).trim().toLowerCase(), d.slice(idx + 1).trim());
      }
    }
    rules.push({ selector, decls });
  }
  return rules;
}

describe('desk-notice look (V3-4 #6512)', () => {
  let deskNotice: Map<string, string> | undefined;

  beforeAll(async () => {
    const specifier = 'node:fs';
    const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
    const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
    const cssText = fs.readFileSync(here + 'desk.css', 'utf8');
    const rules = parseRules(cssText);
    deskNotice = rules.find((r) => r.selector === '.desk-notice')?.decls;
  });

  // Restated for V4-2: var(--xc-warn) (#986814) on the page (#f4f4f6) is 4.42:1, under WCAG AA 4.5:1 (one-look v4,
  // #6360 issuecomment-6686593756 §4); --xc-amber-800 (#805516) keeps the warning hue above AA.
  it('.desk-notice has color: var(--xc-amber-800) and no background declaration', () => {
    expect(deskNotice, '.desk-notice rule exists').toBeDefined();
    expect(deskNotice?.get('color')).toBe('var(--xc-amber-800)');
    expect(deskNotice?.has('background'), '.desk-notice has no background declaration').toBe(false);
    expect(deskNotice?.has('background-color'), '.desk-notice has no background-color declaration').toBe(false);
  });
});
