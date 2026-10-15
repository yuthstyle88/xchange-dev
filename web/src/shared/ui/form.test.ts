// V0-A: useForm (docs/design/xchange-v3-web.md §4.3, §5.2 and §9).
// Written before the implementation. Do not edit, delete or weaken.
import { afterEach, describe, expect, it } from 'vitest';
import { createRoot } from 'solid-js';
import { ApiError } from '../api/client';
import { useForm } from './form';

function input(name: string): HTMLInputElement {
  const el = document.createElement('input');
  el.name = name;
  document.body.appendChild(el);
  return el;
}

afterEach(() => {
  document.body.innerHTML = '';
});

describe('useForm', () => {
  it('focuses and marks the field an ApiError names, and shows its message there', async () => {
    await createRoot(async (dispose) => {
      const form = useForm();
      const user = input('username');
      const password = input('password');
      form.field('username').ref(user);
      form.field('password').ref(password);
      user.focus();

      const message = 'รหัสผ่านต้องมี 12–128 ตัวอักษร';
      await form.submit(async () => {
        throw new ApiError({ code: 'employeePasswordLength', field: 'password', message, status: 400 });
      })();

      expect(document.activeElement).toBe(password);
      expect(password.getAttribute('aria-invalid')).toBe('true');
      expect(user.getAttribute('aria-invalid')).not.toBe('true');
      expect(form.errorFor('password')).toBe(message);
      expect(form.field('password').error()).toBe(message);
      expect(form.field('password').invalid()).toBe(true);
      expect(form.errorFor('username')).toBeUndefined();
      expect(form.formError()).toBeUndefined();
      dispose();
    });
  });

  it('shows a refusal without a field at the form level and marks no field', async () => {
    await createRoot(async (dispose) => {
      const form = useForm();
      const password = input('password');
      form.field('password').ref(password);

      await form.submit(async () => {
        throw new ApiError({ code: 'signIn', message: 'กรุณาเข้าสู่ระบบ', status: 401 });
      })();

      expect(form.formError()).toBe('กรุณาเข้าสู่ระบบ');
      expect(form.errorFor('password')).toBeUndefined();
      expect(password.getAttribute('aria-invalid')).not.toBe('true');
      dispose();
    });
  });

  it('clears the marks of the last refusal when submitted again and answered', async () => {
    await createRoot(async (dispose) => {
      const form = useForm();
      const password = input('password');
      form.field('password').ref(password);

      await form.submit(async () => {
        throw new ApiError({ code: 'x', field: 'password', message: 'ผิด', status: 400 });
      })();
      expect(password.getAttribute('aria-invalid')).toBe('true');

      await form.submit(async () => ({ ok: true }))();
      expect(password.getAttribute('aria-invalid')).not.toBe('true');
      expect(form.errorFor('password')).toBeUndefined();
      expect(form.formError()).toBeUndefined();
      dispose();
    });
  });

  it('a field it does not know is shown at the form level', async () => {
    await createRoot(async (dispose) => {
      const form = useForm();
      form.field('password').ref(input('password'));
      await form.submit(async () => {
        throw new ApiError({ code: 'x', field: 'nickname', message: 'ชื่อเล่นยาวเกิน', status: 400 });
      })();
      expect(form.formError()).toBe('ชื่อเล่นยาวเกิน');
      dispose();
    });
  });

  it('stops the browser from submitting the form itself', async () => {
    await createRoot(async (dispose) => {
      const form = useForm();
      const event = new Event('submit', { cancelable: true });
      await form.submit(async () => undefined)(event);
      expect(event.defaultPrevented).toBe(true);
      dispose();
    });
  });
});

import { setLang } from '../i18n';
import { en } from '../i18n/en';
import { th } from '../i18n/th';

describe('useForm wordedError (#1848)', () => {
  afterEach(() => {
    setLang('th');
  });

  it('row 1: 500 with unknown code shows th error.serverError and hides raw message', async () => {
    await createRoot(async (dispose) => {
      setLang('th');
      const form = useForm();
      await form.submit(async () => {
        throw new ApiError({ status: 500, code: 'dbTimeout', message: 'pool timed out' });
      })();

      expect(form.formError()).toBe(th['error.serverError']);
      expect(form.formError()).not.toContain('pool timed out');
      dispose();
    });
  });

  it('row 2: 500 with keyed code roleNotAllowedHere shows th error.serverError (status wins over keyed code)', async () => {
    await createRoot(async (dispose) => {
      setLang('th');
      const form = useForm();
      await form.submit(async () => {
        throw new ApiError({ status: 500, code: 'roleNotAllowedHere', message: 'raw role message' });
      })();

      expect(form.formError()).toBe(th['error.serverError']);
      dispose();
    });
  });

  it('row 3: 502 with code httpError shows th error.serverError', async () => {
    await createRoot(async (dispose) => {
      setLang('th');
      const form = useForm();
      await form.submit(async () => {
        throw new ApiError({ status: 502, code: 'httpError', message: 'Bad Gateway' });
      })();

      expect(form.formError()).toBe(th['error.serverError']);
      expect(form.formError()).not.toContain('Bad Gateway');
      dispose();
    });
  });

  it('row 4: 422 on registered field email sets field error, leaves formError undefined, and marks/focuses email', async () => {
    await createRoot(async (dispose) => {
      setLang('th');
      const form = useForm();
      const other = input('username');
      const email = input('email');
      form.field('username').ref(other);
      form.field('email').ref(email);
      other.focus();

      await form.submit(async () => {
        throw new ApiError({
          status: 422,
          field: 'email',
          code: 'emailTaken',
          message: 'อีเมลนี้มีบัญชีอยู่แล้ว',
        });
      })();

      expect(form.field('email').error()).toBe('อีเมลนี้มีบัญชีอยู่แล้ว');
      expect(form.errorFor('email')).toBe('อีเมลนี้มีบัญชีอยู่แล้ว');
      expect(form.formError()).toBeUndefined();
      expect(email.getAttribute('aria-invalid')).toBe('true');
      expect(document.activeElement).toBe(email);

      other.focus();
      await form.submit(async () => {
        throw new ApiError({
          status: 422,
          field: 'email',
          code: 'invalidRequest',
          message: 'raw invalid request',
        });
      })();

      expect(form.field('email').error()).toBe(th['error.invalidRequest']);
      expect(form.formError()).toBeUndefined();
      expect(email.getAttribute('aria-invalid')).toBe('true');
      expect(document.activeElement).toBe(email);
      dispose();
    });
  });

  it('row 5: 403 with keyed code roleNotAllowedHere shows th error.roleNotAllowedHere', async () => {
    await createRoot(async (dispose) => {
      setLang('th');
      const form = useForm();
      await form.submit(async () => {
        throw new ApiError({ status: 403, code: 'roleNotAllowedHere', message: 'raw 403' });
      })();

      expect(form.formError()).toBe(th['error.roleNotAllowedHere']);
      dispose();
    });
  });

  it('row 6: 409 with unknown code someUnknownCode shows the server sentence', async () => {
    await createRoot(async (dispose) => {
      setLang('th');
      const form = useForm();
      await form.submit(async () => {
        throw new ApiError({ status: 409, code: 'someUnknownCode', message: 'a server sentence' });
      })();

      expect(form.formError()).toBe('a server sentence');
      dispose();
    });
  });

  it('row 7: 422 on unregistered field notRegistered routes to formError()', async () => {
    await createRoot(async (dispose) => {
      setLang('th');
      const form = useForm();
      form.field('email').ref(input('email'));
      await form.submit(async () => {
        throw new ApiError({ status: 422, field: 'notRegistered', code: 'x', message: 's' });
      })();

      expect(form.formError()).toBe('s');
      expect(form.errorFor('email')).toBeUndefined();
      dispose();
    });
  });

  it('row 8: TypeError (not an ApiError) is rethrown by submit', async () => {
    await createRoot(async (dispose) => {
      const form = useForm();
      await expect(
        form.submit(async () => {
          throw new TypeError('Failed to fetch');
        })(),
      ).rejects.toThrow(TypeError);
      expect(form.formError()).toBeUndefined();
      dispose();
    });
  });

  it('row 9: switching language to en after row 1 resolves formError() to en error.serverError', async () => {
    await createRoot(async (dispose) => {
      setLang('th');
      const form = useForm();
      await form.submit(async () => {
        throw new ApiError({ status: 500, code: 'dbTimeout', message: 'pool timed out' });
      })();

      expect(form.formError()).toBe(th['error.serverError']);
      setLang('en');
      expect(form.formError()).toBe(en['error.serverError']);
      dispose();
    });
  });
});

