// Kiosk light / dark mode (docs/design/xchange-one-look.md §2.6; owner 2026-11-20 "ธีม ไม่สมบูรณ์", "ไม่มี dark mode").
// api/static/kiosk/ is served byte for byte (api/tests/pages_kiosk_test.rs), so these read the files directly.
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

let html = '';
let tokens = '';
let kiosk = '';
let xchange = '';
let attract = '';
let webTheme = '';

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  const dir = here + '../../../api/static/kiosk/';
  html = fs.readFileSync(dir + 'kiosk.html', 'utf8');
  tokens = fs.readFileSync(dir + 'xc-tokens.css', 'utf8');
  kiosk = fs.readFileSync(dir + 'kiosk.css', 'utf8');
  xchange = fs.readFileSync(dir + 'kiosk-xchange.css', 'utf8');
  attract = fs.readFileSync(dir + 'kiosk-attract.css', 'utf8');
  webTheme = fs.readFileSync(here + '../shared/ui/xc-theme.css', 'utf8');
});

const strip = (css: string) => css.replace(/\/\*[\s\S]*?\*\//g, '');

/** The declarations of the dark block in xc-tokens.css. */
function darkBlock(): string {
  const m = /@media screen\{:root\[data-theme="dark"\]\{([^}]*)\}\}/.exec(strip(tokens));
  return m?.[1] ?? '';
}

describe('kiosk mode switch (kiosk.html)', () => {
  it('KD-1 a head script applies the saved choice, else the device setting, before the first stylesheet', () => {
    const head = html.slice(0, html.indexOf('</head>'));
    const script = head.indexOf('<script>(function(){');
    expect(script).toBeGreaterThan(0);
    expect(script).toBeLessThan(head.indexOf('<link rel="stylesheet"'));
    const body = head.slice(script, head.indexOf('</script>', script));
    expect(body).toContain("'xc-theme-mode'");
    expect(body).toContain('prefers-color-scheme: dark');
    expect(body).toContain("setAttribute('data-theme'");
  });

  it('KD-2 the header carries one 48px switch button', () => {
    const header = html.slice(html.indexOf('<header>'), html.indexOf('</header>'));
    expect(header.match(/<button id="theme-switch"[^>]*>/g)).toHaveLength(1);
    expect(header).toMatch(/<button id="theme-switch"[^>]*type="button"[^>]*aria-pressed="false"[^>]*aria-label="[^"]+"/);
    const rule = /body\.customer-kiosk header \.kiosk-theme\{([^}]*)\}/.exec(strip(xchange))?.[1] ?? '';
    for (const p of ['width', 'min-width', 'height', 'min-height']) expect(rule).toMatch(new RegExp(`(^|;)${p}:48px`));
  });
});

describe('kiosk dark values (xc-tokens.css)', () => {
  it('KD-3 the dark block is screen-only on :root[data-theme="dark"] and mirrors every ramp', () => {
    const block = darkBlock();
    expect(block).toContain('color-scheme:dark');
    for (const ramp of ['gray-900', 'gray-600', 'gray-75', 'gray-50', 'gray-0', 'brand-700', 'brand-50', 'green-700', 'red-700']) {
      expect(block).toContain(`--xc-${ramp}:var(--xc-dk-${ramp})`);
    }
    expect(block).toContain('--xc-brand-text:var(--xc-dk-brand-text)');
    expect(block).toContain('--xc-on-brand:var(--xc-dk-on-brand)');
  });

  it('KD-4 its --xc-dk-* values are the staff dark palette, value for value', () => {
    const web = [...webTheme.matchAll(/(--xc-dk-[a-z0-9-]+):\s*([^;]+);/g)];
    expect(web.length).toBeGreaterThan(40);
    for (const [, name, value] of web) {
      expect(new RegExp(`${name}:\\s*([^;]+);`).exec(tokens)?.[1]?.trim(), name).toBe(value!.trim());
    }
  });

  it('KD-5 light mode is unchanged: brand text equals the brand', () => {
    expect(tokens).toMatch(/--xc-brand-text:var\(--xc-brand\);/);
  });
});

describe('kiosk stylesheets avoid the dark-mode traps', () => {
  it('KD-6 brand-coloured text reads --xc-brand-text, never a fill token', () => {
    for (const [name, css] of [
      ['kiosk.css', kiosk],
      ['kiosk-xchange.css', xchange],
      ['kiosk-attract.css', attract],
    ] as const) {
      const bad = strip(css).match(/(^|[^-\w])color:var\(--(xc-brand|xc|brand|pass-red)(,[^)]*)?\)/g) ?? [];
      expect(bad, name).toEqual([]);
    }
  });

  it('KD-7 white-on-brand text never reads a gray ramp step (it turns dark in dark mode)', () => {
    expect(strip(kiosk + xchange + attract)).not.toMatch(/(^|[^-\w])color:var\(--xc-gray-0\)/);
  });

  it('KD-8 the disabled Review rule applies at every width, not only on phones', () => {
    const src = strip(xchange);
    const at = src.indexOf('body.customer-kiosk #review:disabled{');
    expect(at).toBeGreaterThan(0);
    const before = src.slice(0, at);
    const opens = (before.match(/\{/g) ?? []).length;
    const closes = (before.match(/\}/g) ?? []).length;
    expect(opens - closes, 'not nested in an @media block').toBe(0);
  });
});

describe('kiosk text pairs read at 4.5:1 in both modes', () => {
  const lightHex = (name: string): string => {
    const v = new RegExp(`--${name}:\\s*([^;]+);`).exec(tokens)?.[1]?.trim();
    if (!v) throw new Error(`no --${name}`);
    const ref = /^var\(--([a-z0-9-]+)\)$/.exec(v);
    return ref ? lightHex(ref[1]!) : v.toLowerCase();
  };
  const resolve = (name: string, mode: 'light' | 'dark'): string => {
    if (mode === 'dark') {
      const d = new RegExp(`(?:^|;)--${name}:var\\(--(xc-dk-[a-z0-9-]+)\\)`).exec(darkBlock());
      if (d) return lightHex(d[1]!);
      const v = new RegExp(`--${name}:\\s*var\\(--([a-z0-9-]+)\\);`).exec(tokens)?.[1];
      if (v) return resolve(v, mode);
    }
    return lightHex(name);
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

  const PAIRS: Array<[string, string, string]> = [
    ['ink on the page', 'xc-ink', 'xc-canvas'],
    ['ink on a card', 'xc-ink', 'xc-surface'],
    ['muted labels on a card', 'xc-muted', 'xc-surface'],
    ['muted subtitle on the page', 'xc-muted', 'xc-canvas'],
    ['the amount field: ink on the surface', 'xc-ink', 'xc-surface'],
    ['the amount you receive: brand text on its tint', 'xc-brand-text', 'xc-brand-50'],
    ['brand text on a card', 'xc-brand-text', 'xc-surface'],
    ['keypad digits on their key', 'xc-ink', 'xc-gray-50'],
    ['quick amounts on their key', 'xc-ink', 'xc-gray-100'],
    ['white on the Review / primary fill', 'xc-on-brand', 'xc-brand'],
    ['white on the primary hover', 'xc-on-brand', 'xc-brand-strong'],
    ['the disabled Review: grey-600 on grey-100', 'xc-gray-600', 'xc-gray-100'],
  ];

  for (const mode of ['light', 'dark'] as const) {
    it.each(PAIRS)(`${mode}: %s`, (_label, fg, bg) => {
      expect(ratio(resolve(fg, mode), resolve(bg, mode))).toBeGreaterThanOrEqual(4.5);
    });
  }
});
