// OL-8 reuse-field-count (round 13 CSS-reuse sweep, owner: "งาน css รียูส ด้วยนะ"). Three admin screens each copied the
// same "n / max" character-counter rule (margin-top 4px, 12px, muted, right-aligned): approvals `.approval-reason-count`,
// booking-slots `.bs-count`, consent `.xc-admin .xc-content .consent-body-count`. No shared class had that look
// (card.css `.form-hint` has no margin-top and no text-align), so it becomes ONE shared class `.xc-field-count` in
// shared/ui/field-count.css, written with the theme tokens (--xc-space-1 = 4px, --xc-text-sm = 12px: computed look unchanged).
// Written before the implementation by claude/d5c44a22 for the sub-controller claude/1cc51615.
// Do not edit, delete or weaken; add tests in a file of your own.
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
  existsSync(file: string): boolean;
}

let fs: NodeFs;
let here = '';

function read(rel: string): string {
  return fs.readFileSync(here + rel, 'utf8');
}

function stripped(css: string): string {
  return css.replace(/\/\*[\s\S]*?\*\//g, '');
}

/** Every rule in the CSS text as [selectors, declarations]. */
function rules(css: string): Array<[string[], Map<string, string>]> {
  const re = /([^{}]+)\{([^{}]*)\}/g;
  const src = stripped(css);
  const out: Array<[string[], Map<string, string>]> = [];
  let m: RegExpExecArray | null;
  while ((m = re.exec(src)) !== null) {
    const d = new Map<string, string>();
    for (const part of m[2]!.split(';')) {
      const i = part.indexOf(':');
      if (i > 0) d.set(part.slice(0, i).trim().toLowerCase(), part.slice(i + 1).trim().replace(/\s+/g, ' '));
    }
    out.push([m[1]!.split(',').map((s) => s.trim()), d]);
  }
  return out;
}

function decls(css: string, selector: string): Map<string, string> | undefined {
  return rules(css).find(([sel]) => sel.includes(selector))?.[1];
}

const SCREENS = [
  {
    name: 'approvals',
    tsx: '../../screens/admin/approvals/Approvals.tsx',
    css: '../../screens/admin/approvals/approvals.css',
    oldClass: 'approval-reason-count',
    oldSelector: '.approval-reason-count',
    line: '<div class="xc-field-count">{reason().length} / 500</div>',
  },
  {
    name: 'booking-slots',
    tsx: '../../screens/admin/booking-slots/BookingSlots.tsx',
    css: '../../screens/admin/booking-slots/booking-slots.css',
    oldClass: 'bs-count',
    oldSelector: '.bs-count',
    line: '<div class="xc-field-count">{newReason().length} / 200</div>',
  },
  {
    name: 'consent',
    tsx: '../../screens/admin/consent/Consent.tsx',
    css: '../../screens/admin/consent/consent.css',
    oldClass: 'consent-body-count',
    oldSelector: '.xc-admin .xc-content .consent-body-count',
    line: '<div class="xc-field-count">{body().length} / 20000</div>',
  },
] as const;

beforeAll(async () => {
  const specifier = 'node:fs';
  fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
});

describe('shared field counter', () => {
  it('FC-1 field-count.css declares .xc-field-count with exactly the counter look, in tokens', () => {
    expect(fs.existsSync(here + 'field-count.css'), 'shared/ui/field-count.css exists').toBe(true);
    const d = decls(read('field-count.css'), '.xc-field-count');
    expect(d, '.xc-field-count rule').toBeDefined();
    expect(Object.fromEntries(d!)).toEqual({
      'margin-top': 'var(--xc-space-1)',
      'font-size': 'var(--xc-text-sm)',
      color: 'var(--xc-muted)',
      'text-align': 'right',
    });
  });

  it('FC-2 the tokens it uses still resolve to the old literal values (no visible change)', () => {
    const theme = stripped(read('xc-theme.css'));
    expect(theme).toMatch(/--xc-space-1:\s*4px;/);
    expect(theme).toMatch(/--xc-text-sm:\s*12px;/);
  });

  for (const s of SCREENS) {
    it(`FC-3 ${s.name}: the copied counter rule is gone from the screen css`, () => {
      const css = read(s.css);
      expect(decls(css, s.oldSelector), `${s.oldSelector} rule in ${s.css}`).toBeUndefined();
      expect(stripped(css)).not.toContain(s.oldClass);
    });

    it(`FC-4 ${s.name}: the counter carries .xc-field-count and the screen imports the shared file`, () => {
      const tsx = read(s.tsx);
      expect(tsx).toContain(s.line);
      expect(tsx).not.toContain(s.oldClass);
      expect(tsx).toContain("import '../../../shared/ui/field-count.css';");
    });
  }
});
