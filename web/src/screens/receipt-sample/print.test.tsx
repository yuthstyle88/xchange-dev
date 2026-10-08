// V1-receipt-sample (#398): the controller's fixtures stay v2's bytes, and only the receipt reaches the paper (S3,
// §9 "printing"). The screen's styles are every .css file in web/src/screens/receipt-sample/ (read from disk: vitest
// returns CSS modules empty); they are parsed here by the browser's CSS parser and matched against the rendered
// screen, so any selector that hides the right elements is accepted.
// Written before the implementation by the lane controller (claude/7ca80c). Do not edit, delete or weaken.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import type { JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../shared/i18n';

type ReceiptSampleScreen = (props: { print?: () => void }) => JSX.Element;

const screenModules = import.meta.glob<{ ReceiptSample?: ReceiptSampleScreen }>('./ReceiptSample.tsx', {
  eager: true,
});

// sha256 of v2's bytes: receipt.css is the inside of v2's <style> block
// (xchange-v2-source@4d0bdbb:src/online/lib/receipt-html.ts:1); the PNGs are v2's two data-URI images, decoded.
const FIXTURES: Record<string, string> = {
  'receipt.css': '470b5e389a9f6b9f25aec0f9f67aced50c9bc5ffe71a007db615f87af7ab99fb',
  'logo.png': '9149f7e888dfab6e5e65a5c374da5debf306f047ffecff1b2a21aad4999c95ca',
  'qr-telegram.png': '0142cb686c10d518f14a94295edba0018f92503ebf8fcfa81859d8af7c8f6680',
};

interface NodeFs {
  readdirSync(dir: string, options: { recursive: true }): string[];
  readFileSync(file: string, encoding: 'utf8'): string;
  readFileSync(file: string): Uint8Array;
}
interface NodeCrypto {
  createHash(algorithm: 'sha256'): { update(data: Uint8Array): { digest(encoding: 'hex'): string } };
}

let fs: NodeFs;
let crypto: NodeCrypto;
let dir = '';
let screenCss = '';

beforeAll(async () => {
  const fsName = 'node:fs';
  const cryptoName = 'node:crypto';
  fs = (await import(/* @vite-ignore */ fsName)) as NodeFs;
  crypto = (await import(/* @vite-ignore */ cryptoName)) as NodeCrypto;
  dir = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  const files = fs.readdirSync(dir, { recursive: true }).filter((f) => f.endsWith('.css')).sort();
  screenCss = files.map((f) => fs.readFileSync(dir + f, 'utf8')).join('\n');
});

let dispose: (() => void) | undefined;
let style: HTMLStyleElement | undefined;

beforeEach(() => {
  window.localStorage.clear();
  setLang('th');
  vi.stubGlobal(
    'fetch',
    vi.fn(
      async () =>
        new Response(
          '{"version":"1.0.0","build":"0123456789abcdef","builtAt":"2026-10-08T01:02:03Z","channel":"staging"}',
        ),
    ),
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

interface Rules {
  print: CSSStyleRule[];
  screen: CSSStyleRule[];
}

function parsedRules(): Rules {
  style = document.createElement('style');
  style.textContent = screenCss;
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

async function mount(): Promise<HTMLElement> {
  const Screen = screenModules['./ReceiptSample.tsx']?.ReceiptSample;
  expect(Screen, 'web/src/screens/receipt-sample/ReceiptSample.tsx exports function ReceiptSample').toBeTypeOf(
    'function',
  );
  const host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => Screen!({ print: () => {} }), host);
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
  return host;
}

function only(host: HTMLElement, selector: string): Element {
  const found = Array.from(host.querySelectorAll(selector));
  expect(found, `exactly one ${selector}`).toHaveLength(1);
  return found[0]!;
}

const VISIBLE_WITHOUT_TEXT = ['img', 'svg', 'button', 'input', 'select', 'textarea', 'canvas', 'video', 'iframe'];

function bearsContent(el: Element): boolean {
  if (VISIBLE_WITHOUT_TEXT.includes(el.localName)) return true;
  return Array.from(el.childNodes).some((n) => n.nodeType === Node.TEXT_NODE && (n as Text).data.trim() !== '');
}

describe('V1-receipt-sample: fixtures and print', () => {
  it.each(Object.entries(FIXTURES))('%s is v2\'s bytes, unchanged (sha256 %s)', (file, sha) => {
    const bytes = fs.readFileSync(dir + file);
    expect(crypto.createHash('sha256').update(bytes).digest('hex')).toBe(sha);
  });

  it('printing hides everything outside the receipt that shows something: heading, button, tip, app chrome (S3)', async () => {
    const rules = parsedRules();
    const host = await mount();
    const receipt = only(host, 'main.receipt');
    const outside = Array.from(host.querySelectorAll('*')).filter(
      (el) => !receipt.contains(el) && !el.contains(receipt) && bearsContent(el),
    );
    expect(outside.length, 'v2 shows a heading, a print button and a tip outside the receipt').toBeGreaterThanOrEqual(3);
    for (const el of outside) {
      expect(hiddenBy(rules.print, el), `<${el.localName}> "${el.textContent}" is not printed`).toBe(true);
    }
  });

  it('printing keeps the whole receipt: it, its ancestors and everything in it (S3)', async () => {
    const rules = parsedRules();
    const host = await mount();
    const receipt = only(host, 'main.receipt');
    for (const el of [receipt, ...Array.from(receipt.querySelectorAll('*'))]) {
      if (!bearsContent(el)) continue;
      expect(hiddenBy(rules.print, el), `<${el.localName}> "${el.textContent}" is printed`).toBe(false);
    }
  });

  it('on screen the heading, the receipt, the print button and the tip are shown', async () => {
    const rules = parsedRules();
    const host = await mount();
    for (const selector of ['p.screen-heading', 'main.receipt', 'button.print-button', 'p.print-tip']) {
      expect(hiddenBy(rules.screen, only(host, selector)), `${selector} is shown`).toBe(false);
    }
  });
});
