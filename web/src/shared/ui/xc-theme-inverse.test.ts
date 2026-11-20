// Dark mode, second pass (owner 2026-11-20: "ธีม ไม่สมบูรณ์" with a screenshot of the admin THB FLOW card unreadable in dark).
// A surface meant to stay dark under white text must read --xc-inverse, never a ramp value or --xc-ink, because
// dark mode mirrors the ramps (xc-theme-dark.css) and --xc-ink turns light. Pins the rule and the contrast of the pairs.
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: string): string;
  readdirSync(dir: string, opts: { recursive: boolean }): string[];
}
interface NodePath {
  dirname(p: string): string;
  join(...segments: string[]): string;
}

let fs: NodeFs;
let path: NodePath;
let here: string;

beforeAll(async () => {
  fs = (await import(/* @vite-ignore */ 'node:fs' as string)) as unknown as NodeFs;
  path = (await import(/* @vite-ignore */ 'node:path' as string)) as unknown as NodePath;
  here = path.dirname(new URL(import.meta.url).pathname);
});

/** Staff/admin CSS rules (customer pages keep their own look) as [file, rule body]. */
function staffRules(): Array<[string, string]> {
  const src = path.join(here, '..', '..');
  const out: Array<[string, string]> = [];
  for (const rel of fs.readdirSync(src, { recursive: true })) {
    if (!rel.endsWith('.css') || rel.includes('customer') || rel.includes('print')) continue;
    const text = fs.readFileSync(path.join(src, rel), 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
    for (const m of text.matchAll(/\{([^{}]*)\}/g)) out.push([rel, m[1]!]);
  }
  return out;
}

describe('fixed-dark surfaces use --xc-inverse', () => {
  it('no rule paints white-on-brand text over --xc-ink or a dark ramp value', () => {
    const bad = staffRules().filter(([, body]) => {
      const onWhite = /(^|[^-])color:\s*var\(--xc-(on-brand|gray-0|brand-ink)\)/.test(body);
      const darkBg = /background(-color)?:\s*var\(--(xc-ink|ink|xc-gray-(7|8|9)\d{2}|xc-gray-1000)\)/.test(body);
      return onWhite && darkBg;
    });
    expect(bad.map(([f]) => f)).toEqual([]);
  });
});

describe('inverse pairs read at 4.5:1 in both modes', () => {
  let theme = '';
  let dark = '';
  beforeAll(() => {
    theme = fs.readFileSync(path.join(here, 'xc-theme.css'), 'utf8');
    dark = fs.readFileSync(path.join(here, 'xc-theme-dark.css'), 'utf8');
  });
  const hexOf = (name: string): string => {
    const m = theme.match(new RegExp(`--${name}:\\s*(#[0-9a-fA-F]{6})`));
    if (!m) throw new Error(`no value for --${name}`);
    return m[1]!.toLowerCase();
  };
  /** The value a token takes: in light it follows `var(--x)` chains in xc-theme.css; in dark it is its --xc-dk-* mapping. */
  const resolve = (token: string, mode: 'light' | 'dark'): string => {
    if (mode === 'dark') {
      const d = dark.match(new RegExp(`--${token}:\\s*var\\(--(xc-dk-[a-z0-9-]+)\\)`));
      if (d) return hexOf(d[1]!);
      if (token === 'xc-on-brand') return '#ffffff';
    }
    const v = theme.match(new RegExp(`^\\s*--${token}:\\s*([^;]+);`, 'm'));
    if (!v) throw new Error(`no --${token}`);
    const val = v[1]!.trim();
    const ref = val.match(/^var\(--([a-z0-9-]+)\)/);
    return ref ? resolve(ref[1]!, 'light') : val.toLowerCase();
  };
  const lum = (h: string): number => {
    const c = [1, 3, 5]
      .map((i) => parseInt(h.slice(i, i + 2), 16) / 255)
      .map((x) => (x <= 0.03928 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4));
    return 0.2126 * c[0]! + 0.7152 * c[1]! + 0.0722 * c[2]!;
  };
  const ratio = (a: string, b: string): number => {
    const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p);
    return (x! + 0.05) / (y! + 0.05);
  };
  for (const mode of ['light', 'dark'] as const) {
    for (const fg of ['xc-on-brand', 'xc-inverse-accent', 'xc-inverse-muted', 'xc-inverse-faint']) {
      it(`${mode}: --${fg} on --xc-inverse`, () => {
        expect(ratio(resolve(fg, mode), resolve('xc-inverse', mode))).toBeGreaterThanOrEqual(4.5);
      });
    }
  }
});
