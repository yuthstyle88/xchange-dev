// V0-A: PasswordField (docs/design/xchange-v3-web.md §5.2 and §9) — the same component as #378:
// the eye inside the field on the right, the field concealed before submit, its value never read.
// Written before the implementation. Do not edit, delete or weaken.
import { afterEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { useForm } from './form';
import { PasswordField } from './PasswordField';

let dispose: (() => void) | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
});

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
  const formEl = host.querySelector('form')!;
  const input = host.querySelector('input')!;
  const eye = host.querySelector('button')!;
  return { host, formEl, input, eye };
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

describe('PasswordField', () => {
  it('renders one password input named by its field, with the eye inside the same wrapper', () => {
    const { host, input, eye } = mount();
    expect(host.querySelectorAll('input')).toHaveLength(1);
    expect(input.type).toBe('password');
    expect(input.name).toBe('password');
    expect(eye).not.toBeNull();
    expect(eye.parentElement).toBe(input.parentElement);
    expect(eye.getAttribute('type')).toBe('button');
    expect(host.textContent).toContain('รหัสผ่าน');
  });

  it('flips the field between password and text, and says so in Thai', () => {
    const { input, eye } = mount();
    expect(eye.getAttribute('aria-label')).toBe('แสดงรหัสผ่าน');
    expect(eye.getAttribute('aria-pressed')).toBe('false');

    eye.click();
    expect(input.type).toBe('text');
    expect(eye.getAttribute('aria-label')).toBe('ซ่อนรหัสผ่าน');
    expect(eye.getAttribute('aria-pressed')).toBe('true');

    eye.click();
    expect(input.type).toBe('password');
    expect(eye.getAttribute('aria-label')).toBe('แสดงรหัสผ่าน');
    expect(eye.getAttribute('aria-pressed')).toBe('false');
  });

  it('conceals the field when its form is submitted', () => {
    const { formEl, input, eye } = mount();
    eye.click();
    expect(input.type).toBe('text');
    formEl.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    expect(input.type).toBe('password');
    expect(eye.getAttribute('aria-pressed')).toBe('false');
  });

  it('never reads the value through .value', () => {
    const { formEl, input, eye } = mount();
    const errors: unknown[] = [];
    const onError = (e: ErrorEvent) => {
      errors.push(e.error ?? e.message);
      e.preventDefault();
    };
    window.addEventListener('error', onError);
    const trap = trapValue(input);
    try {
      eye.click();
      eye.click();
      eye.click();
      formEl.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
      input.dispatchEvent(new Event('input', { bubbles: true }));
      input.dispatchEvent(new Event('change', { bubbles: true }));
    } finally {
      window.removeEventListener('error', onError);
    }
    expect(trap.reads).toBe(0);
    expect(errors).toEqual([]);
  });
});
