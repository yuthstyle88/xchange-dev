// V1-auth-layout (#531; design docs/design/xchange-v3-web.md §12.21.1): PasswordField's eye looks like the served
// pages' eye (#378, api/web/forms/src/password-field.ts and style.ts): an icon inside the field's right edge, not a text
// button beside it. On signed-out pages only (under .xc-auth); the admin form is not restyled.
// Written before the implementation by the lane controller (claude/608dff). Do not edit, delete or weaken.
//
// PasswordField.test.tsx, PasswordField.validation.test.tsx and shared/i18n/retrofit.test.tsx keep pinning the
// behaviour (flip, conceal on submit, never read .value, aria-label and text from t()); this file pins the markup the
// look hangs on and, with web/src/shared/ui/auth.css applied (read from disk), the look itself.
import { afterEach, beforeAll, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../i18n';
import { useForm } from './form';
import { PasswordField } from './PasswordField';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
  existsSync(file: string): boolean;
}

let authCss: string | undefined;
let dispose: (() => void) | undefined;
let style: HTMLStyleElement | undefined;

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const dir = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  authCss = fs.existsSync(dir + 'auth.css') ? fs.readFileSync(dir + 'auth.css', 'utf8') : undefined;
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  style?.remove();
  style = undefined;
  document.body.innerHTML = '';
  setLang('th');
});

function applyAuthCss() {
  expect(authCss, 'web/src/shared/ui/auth.css exists (§12.21.1)').toBeTypeOf('string');
  style = document.createElement('style');
  style.textContent = authCss!;
  document.head.appendChild(style);
}

/** Mounts one PasswordField in a form, inside `.xc-auth` when `signedOut`. */
function mount(signedOut: boolean) {
  const host = document.createElement('div');
  if (signedOut) host.className = 'xc-auth';
  document.body.appendChild(host);
  dispose = render(() => {
    const form = useForm();
    return (
      <form>
        <PasswordField field={form.field('password')} label="รหัสผ่าน" />
      </form>
    );
  }, host);
  const input = host.querySelector('input')!;
  const eye = host.querySelector('button')!;
  return { host, input, eye };
}

function visuallyHidden(el: Element): boolean {
  const s = getComputedStyle(el);
  if (s.display === 'none') return true;
  return s.position === 'absolute' && (s.width === '1px' || s.clip.replace(/\s/g, '').startsWith('rect(0'));
}

describe('PasswordField — the eye inside the field (§12.21.1, #378)', () => {
  it('the wrapper is .xc-password, and the eye is .xc-reveal, a button, the input\'s next sibling inside it', () => {
    const { input, eye } = mount(false);
    const wrapper = input.parentElement!;
    expect(wrapper.classList.contains('xc-password'), 'the input sits in .xc-password').toBe(true);
    expect(input.nextElementSibling).toBe(eye);
    expect(wrapper.contains(eye)).toBe(true);
    expect(eye.classList.contains('xc-reveal')).toBe(true);
    expect(eye.getAttribute('type')).toBe('button');
  });

  it('the eye keeps its text, in span.xc-reveal-text, and its aria-label and pressed state', () => {
    const { input, eye } = mount(false);
    const text = eye.querySelector('.xc-reveal-text');
    expect(text, 'the eye text sits in span.xc-reveal-text').not.toBeNull();
    expect(text!.textContent?.trim()).toBe('แสดงรหัสผ่าน');
    expect(eye.getAttribute('aria-label')).toBe('แสดงรหัสผ่าน');
    expect(eye.getAttribute('aria-pressed')).toBe('false');
    eye.click();
    expect(input.type).toBe('text');
    expect(eye.querySelector('.xc-reveal-text')?.textContent?.trim()).toBe('ซ่อนรหัสผ่าน');
    expect(eye.getAttribute('aria-label')).toBe('ซ่อนรหัสผ่าน');
    expect(eye.getAttribute('aria-pressed')).toBe('true');
  });

  it('under .xc-auth the eye sits inside the field on the right, and its text is visually hidden', () => {
    applyAuthCss();
    const { input, eye } = mount(true);
    const wrapper = input.parentElement!;
    expect(getComputedStyle(wrapper).position).toBe('relative');
    const e = getComputedStyle(eye);
    expect(e.position).toBe('absolute');
    expect(e.right).toBe('0px');
    const i = getComputedStyle(input);
    expect(i.width).toBe('100%');
    expect(parseFloat(i.paddingRight), 'the input leaves room for the eye').toBeGreaterThanOrEqual(40);
    expect(visuallyHidden(eye.querySelector('.xc-reveal-text')!), 'the eye text is visually hidden').toBe(true);
  });

  it('outside .xc-auth (the admin form) the eye is not restyled', () => {
    applyAuthCss();
    const { input, eye } = mount(false);
    expect(getComputedStyle(input.parentElement!).position).not.toBe('relative');
    expect(getComputedStyle(eye).position).not.toBe('absolute');
    expect(visuallyHidden(eye.querySelector('.xc-reveal-text') ?? eye)).toBe(false);
  });
});
