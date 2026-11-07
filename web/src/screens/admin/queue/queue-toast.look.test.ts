// OL-7 admin-queue-toast (verifier note outside the 138 rows, ~/108-Ting-Ecosystem/.worktrees/xc-verify-138/VERIFY.md
// "Seen outside the 138 rows"): on admin pages the toast "มีคิวลูกค้ารอ … · MAIN" sat over the right end of the white
// header, because queue.css pinned it at `top: 24px` (a v2 layout with no header there). It now sits in the queue corner,
// stacked above the shared queue card: its bottom is the card's own reserve `--xc-queue-card-space` (card height + 36px,
// set by shared/ui/QueueCard.tsx), so it never reaches the header and never lands on the card. Without a card (no
// QueueOperate) it falls back to the card's own 20px corner. Written before the implementation by claude/94389a6b for
// the sub-controller claude/1cc51615. Do not edit, delete or weaken; add tests in a file of your own.
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

let css = '';

/** Declarations of the first rule whose selector list contains `selector` exactly, outside any @media block. */
function decls(source: string, selector: string): Map<string, string> | undefined {
  const top = source.replace(/@media[^{]*\{(?:[^{}]*\{[^{}]*\})*[^{}]*\}/g, '');
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(top)) !== null) {
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
  css = fs.readFileSync(here + 'queue.css', 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
});

describe('admin queue toast placement', () => {
  const sel = '.xc-admin .xc-content .admin-queue-toast';

  it('QT-1 the toast is not pinned to the top of the viewport (it covered the white header)', () => {
    const d = decls(css, sel);
    expect(d, 'the toast rule exists').toBeDefined();
    expect(d!.has('top'), 'no top offset').toBe(false);
  });

  it('QT-2 the toast stands on the queue card: bottom is the card reserve, 20px when there is no card', () => {
    const d = decls(css, sel);
    expect(d!.get('position')).toBe('fixed');
    expect(d!.get('bottom')).toBe('var(--xc-queue-card-space, 20px)');
  });

  it('QT-3 the toast lines up with the card on the right (20px, shared/ui/queue-card.css .queue-card)', () => {
    const d = decls(css, sel);
    expect(d!.get('right')).toBe('20px');
  });

  it('QT-4 unchanged: it stays above page content (z-index 1050) and keeps its card look', () => {
    const d = decls(css, sel);
    expect(d!.get('z-index')).toBe('1050');
    expect(d!.get('border')).toBe('2px solid var(--xc-brand)');
    expect(d!.get('max-width')).toBe('340px');
  });
});
