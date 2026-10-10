// #866: one password field, one look, everywhere. The owner (2026-10-10, admin reset-password dialog):
// "input password ต้อง ทำเป็น component reuse". The eye that #531 put inside the field on signed-out pages
// (`.xc-auth`, auth.css) showed in admin as a grey text button under the field, because its look lived in
// auth.css, scoped to `.xc-auth`, and admin.css's base button rule restyled it. The admin create dialog
// also used a bare `<input type="password">` with no eye at all.
//
// Written before the implementation by the ticket author (claude/b060e9). Do not edit, delete or weaken;
// add tests in a file of your own.
//
// What this file pins:
//   1. The look belongs to the component. `web/src/shared/ui/password-field.css` exists and `PasswordField.tsx`
//      imports it.
//   2. With only password-field.css applied, and no page scope, the eye sits inside the field on the right and
//      its text is visually hidden (the same checks as PasswordField.eye.test.tsx's `.xc-auth` case).
//   3. The same holds in an admin dialog: under `.xc-admin .xc-content`, with admin.css applied as well, whose
//      base button rule (`.xc-admin .xc-content.xc-content button`, admin.css:55) must not win.
//   4. No screen renders a password input of its own: outside PasswordField.tsx, no non-test `.tsx` under
//      web/src has `type="password"`.
// vitest does not load CSS imports, so the stylesheets are read from disk and applied by hand.
import { afterEach, beforeAll, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../i18n';
import { useForm } from './form';
import { PasswordField } from './PasswordField';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
  existsSync(file: string): boolean;
  readdirSync(dir: string): string[];
  statSync(file: string): { isDirectory(): boolean };
}

let fs: NodeFs;
let uiDir: string;
let srcDir: string;
let dispose: (() => void) | undefined;
const styles: HTMLStyleElement[] = [];

beforeAll(async () => {
  const specifier = 'node:fs';
  fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  uiDir = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  srcDir = uiDir.replace(/shared\/ui\/$/, '');
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  for (const s of styles.splice(0)) s.remove();
  document.body.innerHTML = '';
  setLang('th');
});

function read(file: string): string | undefined {
  return fs.existsSync(file) ? fs.readFileSync(file, 'utf8') : undefined;
}

function apply(css: string | undefined, what: string) {
  expect(css, `${what} exists`).toBeTypeOf('string');
  const style = document.createElement('style');
  style.textContent = css!;
  document.head.appendChild(style);
  styles.push(style);
}

/** Mounts one PasswordField in a form, inside the given chain of wrapper classes (outermost first). */
function mount(scopes: string[]) {
  let host: HTMLElement = document.body;
  for (const cls of scopes) {
    const el = document.createElement('div');
    el.className = cls;
    host.appendChild(el);
    host = el;
  }
  const mountPoint = document.createElement('div');
  host.appendChild(mountPoint);
  dispose = render(() => {
    const form = useForm();
    return (
      <form>
        <PasswordField field={form.field('password')} label="รหัสผ่านชั่วคราวใหม่" />
      </form>
    );
  }, mountPoint);
  const input = mountPoint.querySelector('input')!;
  const eye = mountPoint.querySelector('button')!;
  return { input, eye };
}

function visuallyHidden(el: Element): boolean {
  const s = getComputedStyle(el);
  if (s.display === 'none') return true;
  return s.position === 'absolute' && (s.width === '1px' || s.clip.replace(/\s/g, '').startsWith('rect(0'));
}

function expectEyeInside(input: HTMLInputElement, eye: HTMLButtonElement) {
  expect(getComputedStyle(input.parentElement!).position, 'the .xc-password wrapper is the eye\'s anchor').toBe(
    'relative',
  );
  const e = getComputedStyle(eye);
  expect(e.position, 'the eye sits inside the field').toBe('absolute');
  expect(e.right, 'on the right edge').toBe('0px');
  expect(parseFloat(e.paddingLeft || '0') + parseFloat(e.paddingRight || '0'), 'not a padded text button').toBeLessThan(
    32,
  );
  expect(parseFloat(getComputedStyle(input).paddingRight), 'the input leaves room for the eye').toBeGreaterThanOrEqual(
    40,
  );
  expect(visuallyHidden(eye.querySelector('.xc-reveal-text')!), 'the eye text is visually hidden').toBe(true);
}

describe('PasswordField: one look on every page', () => {
  it('its look lives in shared/ui/password-field.css, which PasswordField.tsx imports', () => {
    expect(read(uiDir + 'password-field.css'), 'web/src/shared/ui/password-field.css exists').toBeTypeOf('string');
    const source = read(uiDir + 'PasswordField.tsx') ?? '';
    expect(source, "PasswordField.tsx imports './password-field.css'").toMatch(
      /import\s+['"]\.\/password-field\.css['"]/,
    );
  });

  it('with only password-field.css applied, and no page scope, the eye sits inside the field', () => {
    apply(read(uiDir + 'password-field.css'), 'web/src/shared/ui/password-field.css');
    const { input, eye } = mount([]);
    expectEyeInside(input, eye);
  });

  it("in an admin dialog (.xc-admin .xc-content, admin.css applied too) the eye still sits inside the field; admin's base button rule does not win", () => {
    apply(read(srcDir + 'screens/admin/admin.css'), 'web/src/screens/admin/admin.css');
    apply(read(uiDir + 'password-field.css'), 'web/src/shared/ui/password-field.css');
    const { input, eye } = mount(['xc-admin', 'xc-content']);
    expectEyeInside(input, eye);
    expect(getComputedStyle(eye).minHeight, "not admin.css:55's 44px button").not.toBe('44px');
  });

  it('the eye still works there: it flips the type and its label, and stays a type=button', () => {
    apply(read(srcDir + 'screens/admin/admin.css'), 'web/src/screens/admin/admin.css');
    apply(read(uiDir + 'password-field.css'), 'web/src/shared/ui/password-field.css');
    const { input, eye } = mount(['xc-admin', 'xc-content']);
    expect(eye.getAttribute('type')).toBe('button');
    expect(input.type).toBe('password');
    eye.click();
    expect(input.type).toBe('text');
    expect(eye.getAttribute('aria-label')).toBe('ซ่อนรหัสผ่าน');
  });
});

describe('No screen renders a password input of its own', () => {
  function walk(dir: string): string[] {
    let out: string[] = [];
    for (const name of fs.readdirSync(dir)) {
      const full = dir + name;
      if (fs.statSync(full).isDirectory()) out = out.concat(walk(full + '/'));
      else if (name.endsWith('.tsx') && !/\.test\.tsx$/.test(name)) out.push(full);
    }
    return out;
  }

  it('outside PasswordField.tsx no .tsx under web/src has type="password" (every password field is a PasswordField)', () => {
    const offenders = walk(srcDir)
      .filter((f) => !f.endsWith('shared/ui/PasswordField.tsx'))
      .filter((f) => /type\s*=\s*["'{]\s*["']?password/.test(read(f) ?? ''))
      .map((f) => f.slice(srcDir.length));
    expect(offenders, 'use shared/ui/PasswordField instead').toEqual([]);
  });
});
