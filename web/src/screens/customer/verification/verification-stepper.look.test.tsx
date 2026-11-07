// OL-7 row 125 (verification ~/xc-verify-138 row 125, shot 071): the 4-step stepper is four equal 94 px columns at
// 430 px, but steps 1 and 4 wrap to two lines ("1. เลือกเอกสาร", "4. ผลตรวจ/แก้ไข") while 2 and 3 sit on one.
// The "N. " prefix is typed into each label (VerificationScreen.tsx:298-318), so it eats the width. Fix: the number
// becomes its own round badge above the label, and the label balances its lines.
// Written before the implementation. Do not edit, delete or weaken; add tests in a file of your own.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { setAudience, setLang } from '../../../shared/i18n';
import { th } from './i18n';
import { mount } from './main';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

type Rule = { media: string; selector: string; decls: Map<string, string> };

function parseCss(text: string): Rule[] {
  const src = text.replace(/\/\*[\s\S]*?\*\//g, '');
  const out: Rule[] = [];
  const normSel = (s: string) => s.trim().replace(/\s+/g, ' ').replace(/\s*([>+~(),])\s*/g, '$1');
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(src)) !== null) {
    const decls = new Map<string, string>();
    for (const d of m[2]!.split(';')) {
      const k = d.indexOf(':');
      if (k > 0) decls.set(d.slice(0, k).trim().toLowerCase(), d.slice(k + 1).trim().replace(/\s+/g, ' '));
    }
    for (const sel of m[1]!.split(',')) out.push({ media: '', selector: normSel(sel), decls });
  }
  return out;
}

let rules: Rule[] = [];
let css = '';

function value(selector: string, prop: string): string | undefined {
  for (let i = rules.length - 1; i >= 0; i -= 1) {
    const r = rules[i]!;
    if (r.selector === selector && r.decls.has(prop)) return r.decls.get(prop);
  }
  return undefined;
}

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  css = fs.readFileSync(here + 'verification.css', 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
  rules = parseCss(css.replace(/@media[^{]*\{(?:[^{}]*\{[^{}]*\})*[^{}]*\}/g, ''));
});

const WAIT_OPTS = { timeout: 500, interval: 20 };
let host: HTMLElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  setAudience('customer');
  setLang('th');
  host = document.createElement('div');
  document.body.appendChild(host);
  vi.spyOn(globalThis, 'fetch').mockImplementation(async (input) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const path = new URL(url, 'http://localhost').pathname;
    const body = path.includes('/views/my_profile')
      ? { displayName: 'Customer Test', permissions: ['CustomerSelf', 'SignedIn'] }
      : path.includes('/views/my_verification')
        ? { case: null, readiness: 'NONE' }
        : {};
    return new Response(JSON.stringify(body), { status: 200, headers: { 'content-type': 'application/json' } });
  });
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.restoreAllMocks();
});

describe('VS verification stepper: number badge + balanced label (row 125)', () => {
  it('VS-1 each step is a number badge (hidden from readers; the <ol> numbers it) and a label without "N. "', async () => {
    dispose = mount(host);
    await vi.waitFor(() => expect(host.querySelectorAll('.xc-verification-step')).toHaveLength(4), WAIT_OPTS);
    const labels = [th.stepSelect, th.stepSubmit, th.stepReview, th.stepResult];
    host.querySelectorAll('.xc-verification-step').forEach((li, i) => {
      const num = li.querySelector('.xc-verification-step-num');
      const label = li.querySelector('.xc-verification-step-label');
      expect(num?.textContent, `step ${i + 1} badge`).toBe(String(i + 1));
      expect(num?.getAttribute('aria-hidden'), `step ${i + 1} badge hidden`).toBe('true');
      expect(label?.textContent, `step ${i + 1} label`).toBe(labels[i]);
      expect(li.textContent ?? '', `step ${i + 1} has no typed "N. "`).not.toMatch(/\d\.\s/);
    });
  });

  it('VS-2 the badge is a 24 px circle in the step colour', () => {
    const B = '.xc-verification-step-num';
    expect(value(B, 'display')).toBe('inline-flex');
    expect(value(B, 'align-items')).toBe('center');
    expect(value(B, 'justify-content')).toBe('center');
    expect(value(B, 'width')).toBe('24px');
    expect(value(B, 'height')).toBe('24px');
    expect(value(B, 'border-radius')).toBe('50%');
    expect(value(B, 'border')).toBe('2px solid currentColor');
    expect(value(B, 'font-size')).toBe('0.75rem');
    expect(value(B, 'font-weight')).toBe('700');
  });

  it('VS-3 the active step fills its badge with the brand', () => {
    const A = '.xc-verification-step--active .xc-verification-step-num';
    expect(value(A, 'background')).toBe('var(--xc-brand)');
    expect(value(A, 'border-color')).toBe('var(--xc-brand)');
    expect(value(A, 'color')).toBe('var(--xc-on-brand)');
  });

  it('VS-4 the label balances its lines so neighbouring steps wrap alike', () => {
    expect(value('.xc-verification-step-label', 'text-wrap')).toBe('balance');
    expect(value('.xc-verification-step-label', 'line-height')).toBe('1.3');
  });

  it('VS-5 at ≤400px the label drops to 0.75rem (Thai labels then fit one line in a 375 px phone)', () => {
    const at = css.search(/@media\s*\(max-width:\s*400px\)/);
    expect(at, '@media (max-width: 400px) exists').toBeGreaterThanOrEqual(0);
    const media = parseCss(css.slice(css.indexOf('{', at) + 1));
    const label = media.find((r) => r.selector === '.xc-verification-step-label');
    expect(label?.decls.get('font-size')).toBe('0.75rem');
  });
});
