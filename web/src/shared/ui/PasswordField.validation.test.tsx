// V1-fields (#401): PasswordField takes required and maxLength and keeps every #378 property
// (ticket §7.1b, row F6). Written before the implementation by the lane controller.
// Do not edit, delete or weaken.
import { afterEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { type Form, useForm } from './form';
import { PasswordField } from './PasswordField';

let dispose: (() => void) | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
});

function mount(withRules = true, onSubmit?: (form: Form) => (e?: Event) => Promise<void>) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  let form!: Form;
  dispose = render(() => {
    form = useForm();
    const handler = onSubmit?.(form);
    return (
      <form onSubmit={handler ? (e) => void handler(e) : undefined}>
        {withRules ? (
          <PasswordField field={form.field('password')} label="รหัสผ่าน" required maxLength={128} />
        ) : (
          <PasswordField field={form.field('password')} label="รหัสผ่าน" />
        )}
      </form>
    );
  }, host);
  const formEl = host.querySelector('form')!;
  const input = host.querySelector('input')!;
  const eye = host.querySelector('button')!;
  return { host, form, formEl, input, eye };
}

/** Counts every read of `input.value`, and throws on it. */
function trapValue(input: HTMLInputElement) {
  const trap = { reads: 0 };
  Object.defineProperty(input, 'value', {
    configurable: true,
    get() {
      trap.reads += 1;
      throw new Error('PasswordField read the password through .value');
    },
    set() {
      // writes are not what this test is about
    },
  });
  return trap;
}

describe('PasswordField required and maxLength (#401)', () => {
  it('sets required and maxlength on the input', () => {
    const { input } = mount();
    expect(input.required).toBe(true);
    expect(input.hasAttribute('required')).toBe(true);
    expect(input.getAttribute('maxlength')).toBe('128');
  });

  it('absent props set nothing (F5)', () => {
    const { input } = mount(false);
    expect(input.hasAttribute('required')).toBe(false);
    expect(input.hasAttribute('maxlength')).toBe(false);
  });

  it('keeps one password input with the eye inside the same wrapper (F6)', () => {
    const { host, input, eye } = mount();
    expect(host.querySelectorAll('input')).toHaveLength(1);
    expect(input.type).toBe('password');
    expect(input.name).toBe('password');
    expect(eye.parentElement).toBe(input.parentElement);
    expect(eye.getAttribute('type')).toBe('button');
    expect(eye.getAttribute('aria-label')).toBe('แสดงรหัสผ่าน');
    expect(eye.getAttribute('aria-pressed')).toBe('false');
  });

  it('an empty required password stops the submit, is marked and focused, without reading .value (F6)', async () => {
    const { form, input } = mount();
    const trap = trapValue(input);
    let calls = 0;
    await form.submit(async () => {
      calls += 1;
    })();
    expect(calls).toBe(0);
    expect(input.getAttribute('aria-invalid')).toBe('true');
    expect(document.activeElement).toBe(input);
    expect(form.errorFor('password')).toBe(input.validationMessage);
    expect(trap.reads).toBe(0);
  });

  it('is concealed when the form is submitted even if the check stops the submit (F6)', async () => {
    let calls = 0;
    const { formEl, input, eye } = mount(true, (form) =>
      form.submit(async () => {
        calls += 1;
      }),
    );
    eye.click();
    expect(input.type).toBe('text');
    formEl.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await Promise.resolve();
    expect(input.type).toBe('password');
    expect(eye.getAttribute('aria-pressed')).toBe('false');
    expect(calls).toBe(0);
  });

  it('never reads .value on submit, input or change with the rules set (F6)', async () => {
    const { formEl, input, eye } = mount(true, (form) => form.submit(async () => undefined));
    const errors: unknown[] = [];
    const onError = (e: ErrorEvent) => {
      errors.push(e.error ?? e.message);
      e.preventDefault();
    };
    window.addEventListener('error', onError);
    const trap = trapValue(input);
    try {
      eye.click();
      formEl.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
      await Promise.resolve();
      input.dispatchEvent(new Event('input', { bubbles: true }));
      input.dispatchEvent(new Event('change', { bubbles: true }));
    } finally {
      window.removeEventListener('error', onError);
    }
    expect(trap.reads).toBe(0);
    expect(errors).toEqual([]);
  });
});
