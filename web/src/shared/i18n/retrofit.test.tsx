// V1-i18n (#427): the shared components already on main speak through t() (docs/design/xchange-v3-web.md
// W10 point 6): PasswordField's eye, useForm's server refusals, and AppShell's language switch.
// Written before the implementation by the lane controller. Do not edit, delete or weaken.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiError } from '../api/client';
import { AppShell } from '../ui/AppShell';
import { useForm } from '../ui/form';
import { PasswordField } from '../ui/PasswordField';
import { en } from './en';
import { lang, setLang } from './index';
import { th } from './th';

let dispose: (() => void) | undefined;

beforeEach(() => {
  window.localStorage.clear();
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  setLang('th');
  window.localStorage.clear();
});

function input(name: string): HTMLInputElement {
  const el = document.createElement('input');
  el.name = name;
  document.body.appendChild(el);
  return el;
}

describe('PasswordField speaks through t()', () => {
  function mount() {
    const host = document.createElement('div');
    document.body.appendChild(host);
    dispose = render(() => {
      const form = useForm();
      return (
        <form>
          <PasswordField field={form.field('password')} label="รหัสผ่าน" />
        </form>
      );
    }, host);
    return { input: host.querySelector('input')!, eye: host.querySelector('button')! };
  }

  it('in English the eye says show / hide from the en table, as its label and its text', () => {
    setLang('en');
    const { input: field, eye } = mount();
    expect(eye.getAttribute('aria-label')).toBe(en['field.password.show']);
    expect(eye.textContent).toContain(en['field.password.show']);
    eye.click();
    expect(field.type).toBe('text');
    expect(eye.getAttribute('aria-label')).toBe(en['field.password.hide']);
    expect(eye.textContent).toContain(en['field.password.hide']);
  });

  it('a switch while it is on screen changes its words without changing its state', () => {
    const { input: field, eye } = mount();
    eye.click();
    expect(eye.getAttribute('aria-label')).toBe(th['field.password.hide']);
    setLang('en');
    expect(eye.getAttribute('aria-label')).toBe(en['field.password.hide']);
    expect(eye.getAttribute('aria-pressed')).toBe('true');
    expect(field.type).toBe('text');
    setLang('th');
    expect(eye.getAttribute('aria-label')).toBe(th['field.password.hide']);
  });
});

describe('useForm shows a server refusal in the chosen language', () => {
  it('a known code without a field: the form-level text is the table text, and follows a later switch', async () => {
    await createRoot(async (done) => {
      const form = useForm();
      form.field('username').ref(input('username'));
      await form.submit(async () => {
        throw new ApiError({ code: 'signIn', message: 'กรุณาเข้าสู่ระบบ', status: 401 });
      })();
      expect(form.formError()).toBe(th['error.signIn']);
      setLang('en');
      expect(form.formError()).toBe(en['error.signIn']);
      setLang('th');
      expect(form.formError()).toBe(th['error.signIn']);
      done();
    });
  });

  it('a known code naming a field: the field text is the table text, and follows a later switch', async () => {
    await createRoot(async (done) => {
      setLang('en');
      const form = useForm();
      const user = input('username');
      form.field('username').ref(user);
      await form.submit(async () => {
        throw new ApiError({ code: 'invalidRequest', field: 'username', message: 'คำขอไม่ถูกต้อง', status: 400 });
      })();
      expect(form.errorFor('username')).toBe(en['error.invalidRequest']);
      expect(form.field('username').error()).toBe(en['error.invalidRequest']);
      expect(user.getAttribute('aria-invalid')).toBe('true');
      setLang('th');
      expect(form.errorFor('username')).toBe(th['error.invalidRequest']);
      done();
    });
  });

  it("a code the tables do not know: the server's own message, whatever the language", async () => {
    await createRoot(async (done) => {
      setLang('en');
      const form = useForm();
      form.field('username').ref(input('username'));
      const message = 'ชื่อผู้ใช้นี้มีอยู่แล้ว';
      await form.submit(async () => {
        throw new ApiError({ code: 'usernameTaken', field: 'username', message, status: 400 });
      })();
      expect(form.errorFor('username')).toBe(message);
      setLang('th');
      expect(form.errorFor('username')).toBe(message);
      done();
    });
  });

  it("a refusal naming a field the form does not know is shown at the form level, through the table", async () => {
    await createRoot(async (done) => {
      setLang('en');
      const form = useForm();
      form.field('password').ref(input('password'));
      await form.submit(async () => {
        throw new ApiError({ code: 'ownerOnly', field: 'nickname', message: 'เฉพาะเจ้าของระบบเท่านั้น', status: 403 });
      })();
      expect(form.formError()).toBe(en['error.ownerOnly']);
      done();
    });
  });
});

describe('AppShell carries the language switch', () => {
  function mount() {
    vi.stubGlobal('fetch', vi.fn(async () => Promise.reject(new TypeError('offline'))));
    const host = document.createElement('div');
    document.body.appendChild(host);
    dispose = render(
      () => (
        <AppShell title="เข้าสู่ระบบ">
          <p>body</p>
        </AppShell>
      ),
      host,
    );
    return host;
  }

  it('has one switch, in its header, and it toggles the language of the page', () => {
    const host = mount();
    const inHeader = host.querySelectorAll('header button');
    expect(inHeader).toHaveLength(1);
    expect(host.querySelectorAll('footer button')).toHaveLength(0);
    expect(host.querySelectorAll('main button')).toHaveLength(0);

    (inHeader[0] as HTMLButtonElement).click();
    expect(lang()).toBe('en');
    expect(document.documentElement.lang).toBe('en');
    (inHeader[0] as HTMLButtonElement).click();
    expect(lang()).toBe('th');
  });
});
