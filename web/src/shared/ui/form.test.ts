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
