// OL-8 row 50 (verification of 2026-11-08, shot 024): on a narrow screen the market-source chip rail opens scrolled
// 16px in, so the first chip sits on the panel's left edge. Measured on xc-v3 (build 3458ca66, same rail rules as
// feat/one-look-8) in Chromium at 390px with the TT, SR and BOT chips: scrollLeft 16 at load, first chip inset 0.
// Cause: `.xc-market-sources` bleeds to the panel border (`margin: 0 -16px`) and pads 16px back in, but each chip has
// `scroll-snap-align: start`, and the rail has no scroll-padding, so the snap lines a chip up with the bare scrollport
// edge (the panel border). With `scroll-padding-inline: 16px` the same page loads at scrollLeft 0, first chip inset 16
// (level with the panel title), and scrolled to the end the BoT chip ends 16px inside the panel.
// jsdom has no layout, so the measured numbers are pinned through the rules that produce them.
// Written before the fix by claude/94389a6b for the OL sub-controller claude/1cc51615. Do not edit, delete or weaken.
import { afterEach, beforeAll, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { makePricingSourceDto } from '../../../shared/testing/fixtures/pricing_board';
import { MarketSources } from './MarketSources';

interface NodeFs {
  readFileSync(path: string, encoding: 'utf8'): string;
}

const RAIL = '.xc-admin .xc-content.xc-content .xc-market-sources';
const CHIP = '.xc-admin .xc-content.xc-content .xc-market-source-chip';

function decls(css: string, selector: string): Map<string, string> | undefined {
  const stripped = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  let out: Map<string, string> | undefined;
  while ((m = re.exec(stripped)) !== null) {
    if (!m[1]!.split(',').map((s) => s.trim()).includes(selector)) continue;
    out ??= new Map();
    for (const d of m[2]!.split(';')) {
      const i = d.indexOf(':');
      if (i > 0) out.set(d.slice(0, i).trim().toLowerCase(), d.slice(i + 1).trim());
    }
  }
  return out;
}

/** The inline (left, right) parts of a 1-4 value `padding` / `margin` shorthand. */
function inline(shorthand: string): [string, string] {
  const v = shorthand.trim().split(/\s+/);
  if (v.length === 1) return [v[0]!, v[0]!];
  if (v.length === 2 || v.length === 3) return [v[1]!, v[1]!];
  return [v[3]!, v[1]!];
}

let host: HTMLDivElement | undefined;
let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  host = undefined;
  setLang('th');
});

describe('admin-pricing OL-8 row 50: the chip rail opens with its first chip inset like the title', () => {
  let css = '';
  beforeAll(async () => {
    const specifier = 'node:fs';
    const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
    const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
    css = fs.readFileSync(here + 'market-sources.css', 'utf8');
  });

  it('ME-1 the rail sets scroll-padding-inline: 16px, so a snapped chip keeps the 16px inset', () => {
    const d = decls(css, RAIL);
    expect(d, `rule \`${RAIL}\``).toBeDefined();
    expect(d!.get('scroll-padding-inline')).toBe('var(--xc-space-4)');
  });

  it('ME-2 the snap inset equals the rail padding on both sides, and the bleed equals it too', () => {
    const d = decls(css, RAIL)!;
    const [padL, padR] = inline(d.get('padding') ?? '0');
    const [marL, marR] = inline(d.get('margin') ?? '0');
    expect([padL, padR]).toEqual(['var(--xc-space-4)', 'var(--xc-space-4)']);
    expect([marL, marR]).toEqual(['-16px', '-16px']);
    expect(d.get('scroll-padding-inline')).toBe(padL);
    expect(d.has('scroll-padding'), 'one scroll-padding declaration: scroll-padding-inline').toBe(false);
  });

  it('ME-3 the chips still snap at their start and the rail still scrolls sideways with a thin bar', () => {
    const rail = decls(css, RAIL)!;
    expect(rail.get('overflow-x')).toBe('auto');
    expect(rail.get('scroll-snap-type')).toBe('x proximity');
    expect(rail.get('scrollbar-width')).toBe('thin');
    expect(decls(css, CHIP)?.get('scroll-snap-align')).toBe('start');
  });

  it('ME-4 seeded TT, SR and BOT sources render three chips in that order inside one rail; BOT is last, with no refresh', () => {
    host = document.createElement('div');
    document.body.appendChild(host);
    const sources = (['TT', 'SR', 'BOT'] as const).map((source) =>
      makePricingSourceDto({ source, fetchedAt: '2026-11-08T02:00:00Z' }),
    );
    dispose = render(
      () => <MarketSources sources={sources} canRefresh={true} onRefresh={() => {}} />,
      host,
    );
    const rails = host.querySelectorAll('.xc-market-sources');
    expect(rails).toHaveLength(1);
    const chips = [...rails[0]!.querySelectorAll<HTMLElement>(':scope > .xc-market-source-chip')];
    expect(chips.map((c) => c.dataset.source)).toEqual(['TT', 'SR', 'BOT']);
    expect(chips[2]!.querySelector('.xc-source-refresh-btn')).toBeNull();
    expect(chips[0]!.querySelector('.xc-source-refresh-btn')).not.toBeNull();
  });
});
