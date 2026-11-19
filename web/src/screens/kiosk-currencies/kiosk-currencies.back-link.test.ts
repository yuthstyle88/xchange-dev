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

describe('kiosk-currencies back link (OL-6 L3 #6533)', () => {
  let cssText: string;
  let sourceText: string;
  let rules: Rule[];

  beforeAll(async () => {
    const specifier = 'node:fs';
    const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
    const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
    cssText = fs.readFileSync(here + 'kiosk-currencies.css', 'utf8');
    sourceText = fs.readFileSync(here + 'KioskCurrenciesScreen.tsx', 'utf8');
    rules = parseRules(cssText);
  });

  it('KioskCurrenciesScreen.tsx back link carries class="kc-back" while href and text are preserved', () => {
    expect(sourceText).toContain('<a class="kc-back" href="/">{t(\'back\')}</a>');
  });

  it('KioskCurrenciesScreen.tsx compare link is unchanged', () => {
    expect(sourceText).toContain('<a href={ref().source} target="_blank" rel="noopener noreferrer">');
  });

  it('.xc-kiosk .xc-shell-main .kc-back has brand color, no underline, and font-weight 600', () => {
    const rule = rules.find((r) => r.selector === '.xc-kiosk .xc-shell-main .kc-back');
    expect(rule, '.xc-kiosk .xc-shell-main .kc-back rule exists').toBeDefined();
    expect(rule?.decls.get('color')).toBe('var(--xc-brand-text)');
    expect(rule?.decls.get('text-decoration')).toBe('none');
    expect(rule?.decls.get('font-weight')).toBe('600');
  });

  it('.xc-kiosk .xc-shell-main .kc-back:hover has underline', () => {
    const rule = rules.find((r) => r.selector === '.xc-kiosk .xc-shell-main .kc-back:hover');
    expect(rule, '.xc-kiosk .xc-shell-main .kc-back:hover rule exists').toBeDefined();
    expect(rule?.decls.get('text-decoration')).toBe('underline');
  });
});
