// V1-fields (#401): useForm().submit checks the bound fields' own browser rules before it calls
// the submit function (ticket §7.2, §9, rows F1–F5). Written before the implementation by the
// lane controller. Do not edit, delete or weaken.
import { afterEach, describe, expect, it } from 'vitest';
import { createRoot } from 'solid-js';
import { ApiError } from '../api/client';
import { useForm } from './form';

function input(name: string, attrs: Record<string, string> = {}): HTMLInputElement {
  const el = document.createElement('input');
  el.name = name;
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v);
  document.body.appendChild(el);
  return el;
}

/** A submit function that only counts its calls: no network. */
function fakeSubmit(result: () => Promise<unknown> = async () => ({ ok: true })) {
  const calls = { n: 0 };
  const fn = async () => {
    calls.n += 1;
    return result();
  };
  return { calls, fn };
}

afterEach(() => {
  document.body.innerHTML = '';
});

describe('useForm check before submit (#401)', () => {
  it('an empty required field stops the submit, is marked, focused, and carries the browser message', async () => {
    await createRoot(async (dispose) => {
      const form = useForm();
      const user = input('username', { required: '' });
      form.field('username').ref(user);
      const { calls, fn } = fakeSubmit();

      await form.submit(fn)();

      expect(calls.n).toBe(0);
      expect(form.field('username').invalid()).toBe(true);
      expect(user.getAttribute('aria-invalid')).toBe('true');
      expect(document.activeElement).toBe(user);
      expect(user.validationMessage).not.toBe('');
      expect(form.errorFor('username')).toBe(user.validationMessage);
      expect(form.formError()).toBeUndefined();
      dispose();
    });
  });

  it('the message is the element validationMessage, not text of its own (F2)', async () => {
    await createRoot(async (dispose) => {
      const form = useForm();
      const user = input('username', { required: '' });
      const browserText = 'Please fill out this field.';
      Object.defineProperty(user, 'validationMessage', { configurable: true, get: () => browserText });
      form.field('username').ref(user);
      const { calls, fn } = fakeSubmit();

      await form.submit(fn)();

      expect(calls.n).toBe(0);
      expect(form.errorFor('username')).toBe(browserText);
      dispose();
    });
  });

  it('a value that breaks the pattern stops the submit', async () => {
    await createRoot(async (dispose) => {
      const form = useForm();
      const user = input('username', { pattern: '[a-zA-Z0-9._\\-]{3,40}' });
      user.value = 'ab';
      form.field('username').ref(user);
      const { calls, fn } = fakeSubmit();

      await form.submit(fn)();

      expect(calls.n).toBe(0);
      expect(user.getAttribute('aria-invalid')).toBe('true');
      expect(document.activeElement).toBe(user);
      dispose();
    });
  });

  it('valid input calls the submit function exactly once and marks nothing', async () => {
    await createRoot(async (dispose) => {
      const form = useForm();
      const user = input('username', { required: '', pattern: '[a-zA-Z0-9._\\-]{3,40}', maxlength: '40' });
      user.value = 'cashier.01';
      form.field('username').ref(user);
      const { calls, fn } = fakeSubmit();

      await form.submit(fn)();

      expect(calls.n).toBe(1);
      expect(form.field('username').invalid()).toBe(false);
      expect(user.getAttribute('aria-invalid')).not.toBe('true');
      expect(form.errorFor('username')).toBeUndefined();
      dispose();
    });
  });

  it('two invalid fields: both marked, the first in binding order is focused', async () => {
    await createRoot(async (dispose) => {
      const form = useForm();
      // DOM order: username, rate. Binding order: rate first.
      const user = input('username', { required: '' });
      const rate = input('rate', { type: 'number', required: '' });
      form.field('rate').ref(rate);
      form.field('username').ref(user);
      const { calls, fn } = fakeSubmit();

      await form.submit(fn)();

      expect(calls.n).toBe(0);
      expect(form.field('rate').invalid()).toBe(true);
      expect(form.field('username').invalid()).toBe(true);
      expect(rate.getAttribute('aria-invalid')).toBe('true');
      expect(user.getAttribute('aria-invalid')).toBe('true');
      expect(document.activeElement).toBe(rate);
      dispose();
    });
  });

  it('a disabled field is not checked', async () => {
    await createRoot(async (dispose) => {
      const form = useForm();
      const off = input('note', { required: '', disabled: '' });
      form.field('note').ref(off);
      const { calls, fn } = fakeSubmit();

      await form.submit(fn)();

      expect(calls.n).toBe(1);
      expect(form.field('note').invalid()).toBe(false);
      dispose();
    });
  });

  it('a number under its min is invalid (rangeUnderflow) and stops the submit', async () => {
    await createRoot(async (dispose) => {
      const form = useForm();
      const rate = input('rate', { type: 'number', min: '0.0001', step: '0.0001', max: '1000000', required: '' });
      rate.value = '0';
      form.field('rate').ref(rate);
      const { calls, fn } = fakeSubmit();

      await form.submit(fn)();

      expect(calls.n).toBe(0);
      expect(form.field('rate').invalid()).toBe(true);
      expect(document.activeElement).toBe(rate);
      dispose();
    });
  });

  it('a number over its max is invalid and stops the submit', async () => {
    await createRoot(async (dispose) => {
      const form = useForm();
      const rate = input('rate', { type: 'number', min: '0.0001', step: '0.0001', max: '1000000' });
      rate.value = '1000001';
      form.field('rate').ref(rate);
      const { calls, fn } = fakeSubmit();

      await form.submit(fn)();

      expect(calls.n).toBe(0);
      expect(form.field('rate').invalid()).toBe(true);
      dispose();
    });
  });

  it('33.25 with step 0.0001 is valid and submits', async () => {
    await createRoot(async (dispose) => {
      const form = useForm();
      const rate = input('rate', { type: 'number', min: '0.0001', step: '0.0001', max: '1000000', required: '' });
      rate.value = '33.25';
      form.field('rate').ref(rate);
      const { calls, fn } = fakeSubmit();

      await form.submit(fn)();

      expect(calls.n).toBe(1);
      expect(form.field('rate').invalid()).toBe(false);
      dispose();
    });
  });

  it('a field marked by the check is cleared on input once it is valid, not before', async () => {
    await createRoot(async (dispose) => {
      const form = useForm();
      const user = input('username', { required: '', pattern: '[a-zA-Z0-9._\\-]{3,40}' });
      form.field('username').ref(user);
      const { fn } = fakeSubmit();

      await form.submit(fn)();
      expect(form.field('username').invalid()).toBe(true);

      user.value = 'ab';
      user.dispatchEvent(new Event('input', { bubbles: true }));
      expect(form.field('username').invalid()).toBe(true);
      expect(user.getAttribute('aria-invalid')).toBe('true');

      user.value = 'abc';
      user.dispatchEvent(new Event('input', { bubbles: true }));
      expect(form.field('username').invalid()).toBe(false);
      expect(user.getAttribute('aria-invalid')).not.toBe('true');
      expect(form.errorFor('username')).toBeUndefined();
      dispose();
    });
  });

  it('a field marked by the check is cleared on change once it is valid', async () => {
    await createRoot(async (dispose) => {
      const form = useForm();
      const rate = input('rate', { type: 'number', min: '0.0001', step: '0.0001', required: '' });
      form.field('rate').ref(rate);
      const { fn } = fakeSubmit();

      await form.submit(fn)();
      expect(form.field('rate').invalid()).toBe(true);

      rate.value = '33.25';
      rate.dispatchEvent(new Event('change', { bubbles: true }));
      expect(form.field('rate').invalid()).toBe(false);
      expect(rate.getAttribute('aria-invalid')).not.toBe('true');
      dispose();
    });
  });

  it('after a passing check, a server refusal naming the field is shown on it as before', async () => {
    await createRoot(async (dispose) => {
      const form = useForm();
      const user = input('username', { required: '' });
      const password = input('password', { required: '', maxlength: '128' });
      user.value = 'cashier.01';
      password.value = 'short';
      form.field('username').ref(user);
      form.field('password').ref(password);
      const message = 'รหัสผ่านต้องมี 12–128 ตัวอักษร';
      const { calls, fn } = fakeSubmit(async () => {
        throw new ApiError({ code: 'employeePasswordLength', field: 'password', message, status: 400 });
      });

      await form.submit(fn)();

      expect(calls.n).toBe(1);
      expect(form.errorFor('password')).toBe(message);
      expect(password.getAttribute('aria-invalid')).toBe('true');
      expect(document.activeElement).toBe(password);
      expect(form.field('username').invalid()).toBe(false);
      expect(form.formError()).toBeUndefined();
      dispose();
    });
  });

  it('after a passing check, a server refusal without a field is shown at the form level', async () => {
    await createRoot(async (dispose) => {
      const form = useForm();
      const user = input('username', { required: '' });
      user.value = 'cashier.01';
      form.field('username').ref(user);
      const { calls, fn } = fakeSubmit(async () => {
        throw new ApiError({ code: 'signIn', message: 'กรุณาเข้าสู่ระบบ', status: 401 });
      });

      await form.submit(fn)();

      expect(calls.n).toBe(1);
      expect(form.formError()).toBe('กรุณาเข้าสู่ระบบ');
      expect(form.field('username').invalid()).toBe(false);
      dispose();
    });
  });

  it('a field fixed after a failed check submits on the next try', async () => {
    await createRoot(async (dispose) => {
      const form = useForm();
      const user = input('username', { required: '' });
      form.field('username').ref(user);
      const { calls, fn } = fakeSubmit();

      await form.submit(fn)();
      expect(calls.n).toBe(0);

      user.value = 'cashier.01';
      await form.submit(fn)();
      expect(calls.n).toBe(1);
      expect(form.field('username').invalid()).toBe(false);
      dispose();
    });
  });
});
