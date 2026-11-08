// OL-7 admin-consent-counter (verifier row 82, VERIFY.md: "counter still unstyled: 15px, text-align start (approvals
// counter is 12px right-aligned)"). The "n / 20000" line under the consent body is a bare <div> with no class, so it takes
// the page's 15px body text. It gets its own class and the same look as the approvals counter
// (approvals.css `.approval-reason-count`: margin-top 4px, 12px, muted, right-aligned). Written before the implementation
// by claude/94389a6b for the sub-controller claude/1cc51615. Do not edit, delete or weaken; add tests in a file of your own.
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

let tsx = '';
let css = '';

function decls(selector: string): Map<string, string> | undefined {
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(css)) !== null) {
    if (!m[1]!.split(',').map((s) => s.trim()).includes(selector)) continue;
    const out = new Map<string, string>();
    for (const d of m[2]!.split(';')) {
      const i = d.indexOf(':');
      if (i > 0) out.set(d.slice(0, i).trim().toLowerCase(), d.slice(i + 1).trim().replace(/\s+/g, ' '));
    }
    return out;
  }
  return undefined;
}

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  tsx = fs.readFileSync(here + 'Consent.tsx', 'utf8');
  css = fs.readFileSync(here + 'consent.css', 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
});

describe('consent body counter', () => {
  it('CC-1 the counter carries the class consent-body-count and the same text', () => {
    expect(tsx).toContain('<div class="consent-body-count">{body().length} / 20000</div>');
    expect(tsx).not.toContain('<div>{body().length} / 20000</div>');
  });

  it('CC-2 it looks like the approvals counter: 4px above, 12px, muted, right-aligned', () => {
    const d = decls('.xc-admin .xc-content .consent-body-count');
    expect(d, 'rule in consent.css').toBeDefined();
    expect(d!.get('margin-top')).toBe('4px');
    expect(d!.get('font-size')).toBe('12px');
    expect(d!.get('color')).toBe('var(--xc-muted)');
    expect(d!.get('text-align')).toBe('right');
  });
});
