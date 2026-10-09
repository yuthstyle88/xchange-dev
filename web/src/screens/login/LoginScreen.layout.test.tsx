// V1-auth-layout (#531; design docs/design/xchange-v3-web.md §12.21): the sign-in screen sits in the signed-out frame
// (AuthLayout), looking like v2's login page (xchange-v2-source@4d0bdbb:src/devices-public/login.html:1): the title,
// the brand line and the language switch inside the card; each label a block above its full-width input.
// Owner's order 2026-10-09: "ปรับ ดีไชน์ ให้ เหมือน หน้าlogin อื่น ทำเป็น component reuse".
// Written before the implementation by the lane controller (claude/608dff). Do not edit, delete or weaken.
//
// LoginScreen.test.tsx keeps pinning the behaviour (session, submit, invite claim, errors, field rules); this file
// pins only the layout. Amended by the OR-3 web follow-up (#762, claude/b060e9) only where the old AuthApi
// (status/login) and the `username` label key are gone: the fake below and the identifier label key. auth.css is read from disk (vitest returns CSS imports empty).
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../shared/i18n';
import type { AuthApi } from './auth-api';
import { en, th } from './i18n';
import { LoginScreen } from './LoginScreen';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
  existsSync(file: string): boolean;
}

let authCss: string | undefined;
let loginSource = '';
let dispose: (() => void) | undefined;
let style: HTMLStyleElement | undefined;

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const dir = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  const cssFile = dir + '../../shared/ui/auth.css';
  authCss = fs.existsSync(cssFile) ? fs.readFileSync(cssFile, 'utf8') : undefined;
  loginSource = fs.readFileSync(dir + 'LoginScreen.tsx', 'utf8');
});

beforeEach(() => {
  setLang('th');
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => new Response('{}', { status: 503, headers: { 'Content-Type': 'application/json' } })),
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

async function settle() {
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
}

async function signedOutScreen() {
  const auth = {
    session: async () => ({ user: null }),
    login: async () => {
      throw new Error('login was not expected in this test');
    },
    claim: async () => {
      throw new Error('claim was not expected in this test');
    },
  } as unknown as AuthApi;
  const host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <LoginScreen auth={auth} navigate={() => {}} />, host);
  await settle();
  return host;
}

function applyAuthCss() {
  expect(authCss, 'web/src/shared/ui/auth.css exists (§12.21.1)').toBeTypeOf('string');
  style = document.createElement('style');
  style.textContent = authCss!;
  document.head.appendChild(style);
}

function cardOf(host: HTMLElement): HTMLElement {
  const card = host.querySelector<HTMLElement>('main.xc-auth-card');
  expect(card, "the screen renders through AuthLayout's card (main.xc-auth-card)").not.toBeNull();
  return card!;
}

function input(host: HTMLElement, autocomplete: string): HTMLInputElement {
  const el = host.querySelector<HTMLInputElement>(`input[autocomplete="${autocomplete}"]`);
  if (!el) throw new Error(`no input with autocomplete="${autocomplete}"`);
  return el;
}

function follows(a: Node, b: Node): boolean {
  return (a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING) !== 0;
}

describe('LoginScreen — in the signed-out frame (§12.21)', () => {
  it('LoginScreen.tsx renders AuthLayout, not AppShell', () => {
    expect(loginSource).toMatch(/import\s*\{\s*AuthLayout\s*\}\s*from\s*['"]\.\.\/\.\.\/shared\/ui\/AuthLayout['"]/);
    expect(loginSource).toContain('<AuthLayout');
    expect(loginSource).not.toContain('AppShell');
  });

  it('the title, the brand line, the form and the message line are all inside the card', async () => {
    const host = await signedOutScreen();
    const card = cardOf(host);
    expect(card.querySelector('header h1')?.textContent?.trim()).toBe(th.title);
    expect(card.textContent).toContain('XCHANGE × T.K.');
    expect(card.querySelector('form')).not.toBeNull();
    const message = card.querySelector('#message');
    expect(message, 'the message line #message inside the card').not.toBeNull();
    expect(message!.getAttribute('role')).toBe('alert');
    expect(follows(card.querySelector('form')!, message!) || card.querySelector('form')!.contains(message!)).toBe(true);
  });

  it('no header sits outside the card, and the footer is under it', async () => {
    const host = await signedOutScreen();
    const card = cardOf(host);
    for (const header of Array.from(host.querySelectorAll('header'))) expect(card.contains(header)).toBe(true);
    const footer = host.querySelector('footer');
    expect(footer).not.toBeNull();
    expect(card.contains(footer!)).toBe(false);
  });

  it('nothing on the screen carries an inline style: the look is auth.css', async () => {
    const host = await signedOutScreen();
    const styled = Array.from(host.querySelectorAll('[style]')).map((e) => e.outerHTML.slice(0, 80));
    expect(styled).toEqual([]);
  });

  it('the language switch inside the card switches the screen', async () => {
    const host = await signedOutScreen();
    const card = cardOf(host);
    const sw = Array.from(card.querySelectorAll('header button')).find((b) => b.textContent?.trim() === 'EN');
    expect(sw, 'the EN switch inside the card header').toBeDefined();
    (sw as HTMLButtonElement).click();
    await settle();
    expect(card.querySelector('header h1')?.textContent?.trim()).toBe(en.title);
    expect(host.querySelector(`label[for="${input(host, 'username').id}"]`)?.textContent?.trim()).toBe((en as unknown as Record<string, string>).identifier);
    expect(Array.from(card.querySelectorAll('button')).some((b) => b.textContent?.trim() === en.submit)).toBe(true);
  });
});

describe("LoginScreen — v2's field look once auth.css applies (§12.21.1, login.html:1)", () => {
  it.each([
    ['identifier', 'username'],
    ['password', 'current-password'],
  ])('the %s label is a block above its full-width input', async (_name, autocomplete) => {
    applyAuthCss();
    const host = await signedOutScreen();
    const field = input(host, autocomplete);
    const label = host.querySelector<HTMLLabelElement>(`label[for="${field.id}"]`);
    expect(label, `a label for #${field.id}`).not.toBeNull();
    expect(follows(label!, field), 'the label comes before its input').toBe(true);
    expect(getComputedStyle(label!).display).toBe('block');
    expect(getComputedStyle(field).width).toBe('100%');
    expect(getComputedStyle(field).boxSizing).toBe('border-box');
  });

  it("the identifier field's wrapper is TextField's .xc-field", async () => {
    const host = await signedOutScreen();
    expect(input(host, 'username').closest('.xc-field'), 'TextField wrapper class xc-field').not.toBeNull();
  });

  it('the submit button is full width', async () => {
    applyAuthCss();
    const host = await signedOutScreen();
    const submit = host.querySelector<HTMLButtonElement>('button[type="submit"]');
    expect(submit).not.toBeNull();
    expect(getComputedStyle(submit!).width).toBe('100%');
  });
});
