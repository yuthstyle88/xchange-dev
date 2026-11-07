// OL-7 admin-alert-settings (verifier row 93, VERIFY.md: "card still narrower than the content column (x 286–925, 640px
// vs ~955px elsewhere)"). `.alert-settings-container { max-width: 640px }` capped the only card on the page. Nothing in
// alert-settings.css narrows the container or the card any more, so the card spans the admin content column like the
// other admin cards. Written before the implementation by claude/94389a6b for the sub-controller claude/1cc51615.
// Do not edit, delete or weaken; add tests in a file of your own.
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

let css = '';

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  css = fs.readFileSync(here + 'alert-settings.css', 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
});

/** Every declaration block whose selector list names `cls` as its last compound (the element itself, not a child). */
function blocksFor(cls: string): string[] {
  return [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)]
    .filter((m) => m[1]!.split(',').some((s) => new RegExp(`\\.${cls}$`).test(s.trim())))
    .map((m) => m[2]!);
}

describe('alert-settings card width', () => {
  it('AW-1 the container sets no max-width or width', () => {
    for (const body of blocksFor('alert-settings-container')) {
      expect(body).not.toMatch(/(^|;|\s)(max-)?width\s*:/);
    }
  });

  it('AW-2 the card sets no max-width or width', () => {
    for (const body of blocksFor('alert-settings-card')) {
      expect(body).not.toMatch(/(^|;|\s)(max-)?width\s*:/);
    }
  });
});
