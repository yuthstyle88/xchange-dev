// OL-6 wave B (cust-copy-link, audit #564 issuecomment-6702970101): the page carries the brand; a failed link offers a way home.
// Written before the implementation. Do not edit, delete or weaken; add tests in a file of your own.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import { CopyLink } from './CopyLink';

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
  rules = parseCss(fs.readFileSync(here + 'copy-link.css', 'utf8'));
});

const HOME = 'a.xc-c-primary[href="/app"]';

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));
}

describe('cust-copy-link: look', () => {
  it('CL-1 the logo is a block centred above the title', () => {
    expect(value('.xc-copy-link-logo', 'display')).toBe('block');
    expect(value('.xc-copy-link-logo', 'width')).toBe('64px');
    expect(value('.xc-copy-link-logo', 'height')).toBe('44px');
    expect(value('.xc-copy-link-logo', 'object-fit')).toBe('contain');
    expect(value('.xc-copy-link-logo', 'margin')).toBe('0 auto 1rem');
  });
});

describe('cust-copy-link: screen', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.restoreAllMocks();
  });

  function answering(status: number, body: unknown) {
    return vi.fn(async () => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } }));
  }

  it('CL-2 no fragment: the invalid line, the logo and a way home', async () => {
    dispose = render(() => <CopyLink fragment="" fetch={answering(500, {})} />, host);
    await flush();
    expect(host.querySelector('.xc-copy-link-error')?.textContent).toBe('ลิงก์ไม่ถูกต้อง');
    expect(host.querySelector('img.xc-copy-link-logo')?.getAttribute('alt')).toBe('XCHANGE Currency Exchange');
    expect(host.querySelector(HOME)?.textContent).toBe('กลับหน้าแรก');
  });

  it('CL-3 expired link: a way home', async () => {
    dispose = render(() => <CopyLink fragment="v1.x" fetch={answering(410, { error: { code: 'gone', message: 'gone' } })} />, host);
    await flush();
    expect(host.querySelector('.xc-copy-link-error')?.textContent).toBe('ลิงก์นี้หมดอายุแล้ว');
    expect(host.querySelector(HOME)).not.toBeNull();
  });

  it('CL-4 load failure: a way home', async () => {
    dispose = render(() => <CopyLink fragment="v1.x" fetch={answering(500, { error: { code: 'x', message: 'x' } })} />, host);
    await flush();
    expect(host.querySelector('.xc-copy-link-error')?.textContent).toBe('โหลดข้อมูลไม่สำเร็จ');
    expect(host.querySelector(HOME)).not.toBeNull();
  });

  it('CL-5 a good link: the logo and no way-home button', async () => {
    const view = { subject: 'webRequest', id: 'r', branch: 'B', rows: [], status: 'WAITING', qr: 'https://x/q#t', expiresAt: '2026-10-23T10:00:00Z' };
    dispose = render(() => <CopyLink fragment="v1.x" fetch={answering(200, view)} />, host);
    await flush();
    expect(host.querySelector('[data-copy-qr]')).not.toBeNull();
    expect(host.querySelector('img.xc-copy-link-logo')).not.toBeNull();
    expect(host.querySelector(HOME)).toBeNull();
  });
});
