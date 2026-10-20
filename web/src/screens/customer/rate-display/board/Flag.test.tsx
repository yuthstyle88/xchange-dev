// P01 v2 RB-10 (#3571): tests for vendored flag-icons and the code-box fallback.
import { afterEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { Flag } from './Flag';
import { flagUrl } from './flag-code';

const fsName = 'node:fs';
const pathName = 'node:path';

const disposers: Array<() => void> = [];

function mountFlag(currency: string): HTMLElement {
  const container = document.createElement('div');
  document.body.appendChild(container);
  disposers.push(render(() => <Flag currency={currency} />, container));
  return container;
}

afterEach(() => {
  while (disposers.length) disposers.pop()?.();
  document.body.innerHTML = '';
});

describe('RB-10 flags (#3571)', () => {
  it('row 1: USD / EUR / JPY resolve to URLs ending us.svg / eu.svg / jp.svg', () => {
    expect(flagUrl('USD')).toMatch(/\/us\.svg(\?.*)?$/);
    expect(flagUrl('EUR')).toMatch(/\/eu\.svg(\?.*)?$/);
    expect(flagUrl('JPY')).toMatch(/\/jp\.svg(\?.*)?$/);
  });

  it('row 2: X* codes (XAU, XOF, XAF, XCD, XCG, XPF) return null while country codes resolve', () => {
    expect(flagUrl('USD')).not.toBeNull();
    expect(flagUrl('XAU')).toBeNull();
    expect(flagUrl('XOF')).toBeNull();
    expect(flagUrl('XAF')).toBeNull();
    expect(flagUrl('XCD')).toBeNull();
    expect(flagUrl('XCG')).toBeNull();
    expect(flagUrl('XPF')).toBeNull();
  });

  it('row 3: a code with no vendored file returns null while a vendored code resolves', () => {
    expect(flagUrl('THB')).toMatch(/\/th\.svg(\?.*)?$/);
    expect(flagUrl('ZZZ')).toBeNull();
    expect(flagUrl('QZZ')).toBeNull();
    expect(flagUrl('')).toBeNull();
  });

  it('row 4: Flag with a URL renders an img with alt="" and no code box', () => {
    const el = mountFlag('USD');
    const img = el.querySelector<HTMLImageElement>('img');
    expect(img).not.toBeNull();
    expect(img!.getAttribute('alt')).toBe('');
    expect(img!.getAttribute('src')).toMatch(/\/us\.svg(\?.*)?$/);
    expect(el.querySelector('.xc-rb-code-box')).toBeNull();
  });

  it('row 5: image error replaces the img with a .xc-rb-code-box showing the currency code', () => {
    const el = mountFlag('USD');
    const img = el.querySelector<HTMLImageElement>('img');
    expect(img, 'initial img before error').not.toBeNull();

    img!.dispatchEvent(new Event('error'));

    expect(el.querySelector('img')).toBeNull();
    const box = el.querySelector<HTMLElement>('.xc-rb-code-box');
    expect(box).not.toBeNull();
    expect(box!.textContent).toBe('USD');
  });

  it('row 6: flags/LICENSE contains the MIT licence text and vendored flags are wired', async () => {
    const fs = (await import(/* @vite-ignore */ fsName)) as {
      readFileSync(p: string, enc: string): string;
    };
    const path = (await import(/* @vite-ignore */ pathName)) as {
      dirname(p: string): string;
      join(...p: string[]): string;
    };
    const here = path.dirname(new URL(import.meta.url).pathname);
    const license = fs.readFileSync(path.join(here, 'flags', 'LICENSE'), 'utf8');
    expect(license).toContain('MIT License');
    expect(license).toContain('Permission is hereby granted, free of charge');
    expect(flagUrl('GBP')).toMatch(/\/gb\.svg(\?.*)?$/);
  });
});
