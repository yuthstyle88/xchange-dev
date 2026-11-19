// OL-6 wave B kiosk-queue: back link and page gutter. Written before the implementation by the test-writer
// claude/94389a6b on the sub-controller claude/1cc51615's order. Do not edit, delete or weaken.
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
    const decls = new Map<string, string>();
    for (const d of m[2]!.split(';')) {
      const idx = d.indexOf(':');
      if (idx > 0) decls.set(d.slice(0, idx).trim().toLowerCase(), d.slice(idx + 1).trim());
    }
    rules.push({ selector: m[1]!.trim().replace(/\s+/g, ' '), decls });
  }
  return rules;
}

describe('kiosk-queue look (OL-6 wave B)', () => {
  let screen: string;
  let rules: Rule[];
  let panelRules: Rule[];
  const rule = (list: Rule[], selector: string) => list.find((r) => r.selector === selector);

  beforeAll(async () => {
    const specifier = 'node:fs';
    const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
    const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
    screen = fs.readFileSync(here + 'KioskQueueScreen.tsx', 'utf8');
    rules = parseRules(fs.readFileSync(here + 'kiosk-queue.css', 'utf8'));
    panelRules = parseRules(fs.readFileSync(here + '../../shared/ui/queue-panel.css', 'utf8'));
  });

  it('KQ-1: the back link carries class kq-back; href and text are unchanged', () => {
    expect(screen).toContain('<a class="kq-back" href="/">{t(\'back\')}</a>');
  });

  it('KQ-2: .kq-back is brand coloured, not underlined, weight 600', () => {
    const r = rule(rules, '.xc-shell-main .queue-page .kq-back');
    expect(r, 'rule exists').toBeDefined();
    expect(r?.decls.get('color')).toBe('var(--xc-brand-text)');
    expect(r?.decls.get('text-decoration')).toBe('none');
    expect(r?.decls.get('font-weight')).toBe('600');
  });

  it('KQ-3: .kq-back underlines on hover', () => {
    expect(rule(rules, '.xc-shell-main .queue-page .kq-back:hover')?.decls.get('text-decoration')).toBe('underline');
  });

  it('KQ-4: the shell main gives the page a 16px side gutter', () => {
    expect(rule(rules, '.xc-shell-main:has(> .queue-page)')?.decls.get('padding')).toBe('0 16px');
  });

  it('KQ-5: .queue-page adds no side padding on top of the gutter (now 28px at 430: 8 + 20)', () => {
    expect(rule(panelRules, '.queue-page')?.decls.get('padding')).toBe('20px 0');
  });
});
