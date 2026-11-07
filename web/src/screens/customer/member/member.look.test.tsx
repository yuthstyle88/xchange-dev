// OL-6 wave B (cust-member, audit #564 issuecomment-6702970101): page subtitle; the points-unavailable line reads as a note.
// Written before the implementation. Do not edit, delete or weaken; add tests in a file of your own.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../shared/api/client';
import { setAudience, setLang } from '../../../shared/i18n';
import { MemberScreen } from './Member';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

type Rule = { media: string; selector: string; decls: Map<string, string> };

function parseCss(text: string): Rule[] {
  const src = text.replace(/\/\*[\s\S]*?\*\//g, '');
  const out: Rule[] = [];
  const norm = (s: string) => s.replace(/\s+/g, '');
  const normSel = (s: string) => s.trim().replace(/\s+/g, ' ').replace(/\s*([>+~(),])\s*/g, '$1');
  const block = (body: string, media: string) => {
    const re = /([^{}]+)\{([^{}]*)\}/g;
    let m: RegExpExecArray | null;
    while ((m = re.exec(body)) !== null) {
      const decls = new Map<string, string>();
      for (const d of m[2]!.split(';')) {
        const k = d.indexOf(':');
        if (k > 0) decls.set(d.slice(0, k).trim().toLowerCase(), d.slice(k + 1).trim().replace(/\s+/g, ' '));
      }
      for (const sel of m[1]!.split(',')) out.push({ media, selector: normSel(sel), decls });
    }
  };
  let rest = '';
  let i = 0;
  while (i < src.length) {
    const at = src.indexOf('@media', i);
    if (at < 0) {
      rest += src.slice(i);
      break;
    }
    rest += src.slice(i, at);
    const open = src.indexOf('{', at);
    let depth = 1;
    let j = open + 1;
    while (j < src.length && depth > 0) {
      if (src[j] === '{') depth += 1;
      if (src[j] === '}') depth -= 1;
      j += 1;
    }
    block(src.slice(open + 1, j - 1), norm(src.slice(at + 6, open)));
    i = j;
  }
  block(rest, '');
  return out;
}

let rules: Rule[] = [];

/** The value the stylesheet's last rule for this exact selector gives the property, inside `media` when named. */
function value(selector: string, prop: string, media = ''): string | undefined {
  const m = media.replace(/\s+/g, '');
  for (let i = rules.length - 1; i >= 0; i -= 1) {
    const r = rules[i]!;
    if (r.selector === selector && r.media === m && r.decls.has(prop)) return r.decls.get(prop);
  }
  return undefined;
}

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  rules = parseCss(fs.readFileSync(here + 'member.css', 'utf8'));
});

describe('cust-member: look', () => {
  it('ME-1 the header stacks the title over the subtitle', () => {
    expect(value('.xc-member-header', 'flex-direction')).toBe('column');
    expect(value('.xc-member-header', 'align-items')).toBe('flex-start');
    expect(value('.xc-member-header', 'gap')).toBe('0.25rem');
  });

  it('ME-2 the subtitle is the muted page subtitle', () => {
    expect(value('.xc-member-subtitle', 'margin')).toBe('0');
    expect(value('.xc-member-subtitle', 'font-size')).toBe('0.875rem');
    expect(value('.xc-member-subtitle', 'color')).toBe('var(--xc-text-muted)');
  });

  it('ME-3 points-unavailable is a muted, centred note', () => {
    expect(value('.xc-member-points-unavailable', 'color')).toBe('var(--xc-text-muted)');
    expect(value('.xc-member-points-unavailable', 'font-size')).toBe('0.875rem');
    expect(value('.xc-member-points-unavailable', 'text-align')).toBe('center');
  });
});

describe('cust-member: screen', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
    setAudience('customer');
    setLang('th');
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.restoreAllMocks();
  });

  it('ME-4 the subtitle sits in the header', async () => {
    const proto = ApiClient.prototype as unknown as { viewBare: (name: string) => Promise<unknown> };
    vi.spyOn(proto, 'viewBare').mockResolvedValue({
      tier: null,
      effectiveUntil: null,
      points: null,
      pointsUnavailable: true,
      nextExpiry: null,
      progress: null,
    });
    dispose = render(() => <MemberScreen />, host);
    await new Promise((r) => setTimeout(r, 20));
    expect(host.querySelector('.xc-member-header p.xc-member-subtitle')?.textContent).toBe(
      'ระดับสมาชิกและคะแนนสะสมของคุณ',
    );
  });
});
