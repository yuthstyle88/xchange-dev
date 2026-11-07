// OL-7 devices-tag (verifier row 97, VERIFY.md: "Thai badge ยืนยันโดยพนักงาน still letter-spacing 0.7px (Latin eyebrows
// normal)"). devices.css gives `.tag` 0.7px tracking for the Latin labels (SIMULATOR, SCAN / IMPORT); Thai must not be
// tracked. The eyebrow already has its :lang(th) reset (DV-3); the tag gets the same. As with the eyebrow (admin A3-6), a
// Latin tag inside the Thai UI loses its tracking too; accepted. Written before the implementation by claude/94389a6b for
// the sub-controller claude/1cc51615. Do not edit, delete or weaken; add tests in a file of your own.
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

let css = '';

function decls(selector: string): Map<string, string> | undefined {
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(css)) !== null) {
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
  css = fs.readFileSync(here + 'devices.css', 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
});

const M = '.xc-devices .xc-shell-main';

describe('devices tags in Thai', () => {
  it('DT-1 a Thai tag has no letter-spacing', () => {
    expect(decls(`${M} .tag:lang(th)`)?.get('letter-spacing')).toBe('0');
  });

  it('DT-2 unchanged: the tag keeps its 0.7px tracking otherwise', () => {
    expect(decls(`${M} .tag`)?.get('letter-spacing')).toBe('.7px');
  });
});
