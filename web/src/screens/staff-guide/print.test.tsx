// V1-staff-guide (#397): v2's styles (G9) and what prints (R3, §9 "printing"). The screen's styles are every .css
// file in web/src/screens/staff-guide/ (read from disk: vitest returns CSS modules empty); they are parsed here by the
// browser's CSS parser and matched against the rendered screen, so any selector that hides the right elements is accepted.
// Written before the implementation by the lane controller (claude/7ca80c). Do not edit, delete or weaken.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import type { JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../shared/i18n';

type StaffGuideScreen = (props: { print?: () => void }) => JSX.Element;

const screenModules = import.meta.glob<{ StaffGuide?: StaffGuideScreen }>('./StaffGuide.tsx', { eager: true });

const TITLE = 'คู่มือพนักงาน XC Desk v2';
const BACK = 'กลับแอดมิน';
const PRINT = 'พิมพ์คู่มือ';

// G9, xchange-v2-source@4d0bdbb:src/devices-public/staff-guide.html:1 (the print rule is checked by matching below).
const V2_RULES = [
  'body{max-width:880px;margin:40px auto;padding:20px;font:18px/1.7 sans-serif;color:#242424}',
  'h1,h2{color:#a5003c}',
  'section{border-bottom:1px solid #ddd;padding-bottom:20px}',
  'a{color:#a5003c}',
];

let dispose: (() => void) | undefined;
let style: HTMLStyleElement | undefined;

beforeEach(() => {
  window.localStorage.clear();
  setLang('th');
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => new Response('{"version":"1.0.0","build":"0123456789","builtAt":"x","channel":"staging"}')),
  );
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  style?.remove();
  style = undefined;
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  setLang('th');
});

interface NodeFs {
  readdirSync(dir: string, options: { recursive: true }): string[];
  readFileSync(file: string, encoding: 'utf8'): string;
}

let screenCss = '';

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const dir = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  const files = fs.readdirSync(dir, { recursive: true }).filter((f) => f.endsWith('.css')).sort();
  screenCss = files.map((f) => fs.readFileSync(dir + f, 'utf8')).join('\n');
});

function allCss(): string {
  expect(screenCss.length, 'web/src/screens/staff-guide/ has CSS (a .css file in the folder)').toBeGreaterThan(0);
  return screenCss;
}

function compact(css: string): string {
  return css.replace(/\s+/g, '').replace(/;}/g, '}');
}

interface Rules {
  print: CSSStyleRule[];
  screen: CSSStyleRule[];
}

function parsedRules(): Rules {
  style = document.createElement('style');
  style.textContent = allCss();
  document.head.appendChild(style);
  const rules: Rules = { print: [], screen: [] };
  const walk = (list: CSSRuleList, media: 'print' | 'screen' | 'both') => {
    for (const rule of Array.from(list)) {
      if (rule.type === CSSRule.STYLE_RULE) {
        if (media !== 'screen') rules.print.push(rule as CSSStyleRule);
        if (media !== 'print') rules.screen.push(rule as CSSStyleRule);
      } else if (rule.type === CSSRule.MEDIA_RULE) {
        const text = (rule as CSSMediaRule).media.mediaText.toLowerCase();
        const kind = /\bprint\b/.test(text) ? 'print' : /\bscreen\b/.test(text) ? 'screen' : 'both';
        walk((rule as CSSMediaRule).cssRules, kind);
      }
    }
  };
  walk(style.sheet!.cssRules, 'both');
  return rules;
}

function matches(el: Element, selector: string): boolean {
  try {
    return el.matches(selector);
  } catch {
    return false;
  }
}

/** True when `el` or an ancestor is matched by one of `rules` that sets display:none. */
function hiddenBy(rules: CSSStyleRule[], el: Element): boolean {
  for (let node: Element | null = el; node; node = node.parentElement) {
    for (const rule of rules) {
      if (rule.style.display === 'none' && matches(node, rule.selectorText)) return true;
    }
  }
  return false;
}

function mount(): HTMLElement {
  const Screen = screenModules['./StaffGuide.tsx']?.StaffGuide;
  expect(Screen, 'web/src/screens/staff-guide/StaffGuide.tsx exports function StaffGuide').toBeTypeOf('function');
  const host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => Screen!({ print: () => {} }), host);
  return host;
}

function one<T extends Element>(list: T[], what: string): T {
  expect(list, what).toHaveLength(1);
  return list[0]!;
}

function navLine(host: HTMLElement): Element[] {
  const link = one(
    Array.from(host.querySelectorAll('a')).filter((a) => a.textContent === BACK),
    `one link reads ${BACK}`,
  );
  const button = one(
    Array.from(host.querySelectorAll('button')).filter((b) => b.textContent === PRINT),
    `one button reads ${PRINT}`,
  );
  return [link, button];
}

describe('V1-staff-guide: styles and print', () => {
  it("carries v2's rules over: body, h1/h2, section and link styles (G9, whitespace-insensitive)", () => {
    const css = compact(allCss());
    for (const rule of V2_RULES) expect(css, `the screen's CSS has ${rule}`).toContain(compact(rule));
  });

  it('printing hides the nav line (link and print button) and the footer (R3)', () => {
    const rules = parsedRules();
    const host = mount();
    for (const el of navLine(host)) {
      expect(hiddenBy(rules.print, el), `${el.tagName} ${el.textContent} is not printed`).toBe(true);
    }
    const footers = Array.from(host.querySelectorAll('footer'));
    expect(footers.length, 'the screen has the AppShell footer').toBeGreaterThan(0);
    for (const footer of footers) expect(hiddenBy(rules.print, footer), 'the footer is not printed').toBe(true);
  });

  it('printing keeps the title and the six parts', () => {
    const rules = parsedRules();
    const host = mount();
    const h1 = one(
      Array.from(host.querySelectorAll('h1')).filter((h) => h.textContent === TITLE),
      'one h1 with the title',
    );
    expect(hiddenBy(rules.print, h1), 'the title is printed').toBe(false);
    const sections = Array.from(host.querySelectorAll('section'));
    expect(sections).toHaveLength(6);
    for (const section of sections) {
      expect(hiddenBy(rules.print, section), `${section.querySelector('h2')?.textContent} is printed`).toBe(false);
    }
  });

  it('on screen the nav line, the title and the six parts are shown', () => {
    const rules = parsedRules();
    const host = mount();
    for (const el of navLine(host)) expect(hiddenBy(rules.screen, el), `${el.textContent} is shown`).toBe(false);
    for (const el of Array.from(host.querySelectorAll('h1, section'))) {
      expect(hiddenBy(rules.screen, el), `${el.tagName} is shown`).toBe(false);
    }
  });
});
