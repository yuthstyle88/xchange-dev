// V0-A: TextField with useForm (docs/design/xchange-v3-web.md §4.3, §5.2).
// Written before the implementation. Do not edit, delete or weaken.
import { afterEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { ApiError } from '../api/client';
import { type Form, useForm } from './form';
import { TextField } from './TextField';

let dispose: (() => void) | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
});

function mount() {
  const host = document.createElement('div');
  document.body.appendChild(host);
  let form!: Form;
  dispose = render(() => {
    form = useForm();
    return (
      <form>
        <TextField field={form.field('username')} label="ชื่อผู้ใช้" />
      </form>
    );
  }, host);
  return { host, form, input: host.querySelector('input')! };
}

describe('TextField', () => {
  it('renders a labelled input named by its field', () => {
    const { host, input } = mount();
    expect(input.name).toBe('username');
    const label = host.querySelector('label');
    expect(label?.textContent).toContain('ชื่อผู้ใช้');
    expect(label?.control ?? null).toBe(input);
  });

  it('shows the refusal that names it beside it, and marks it', async () => {
    const { host, form, input } = mount();
    const message = 'ชื่อผู้ใช้นี้มีอยู่แล้ว';
    await form.submit(async () => {
      throw new ApiError({ code: 'usernameTaken', field: 'username', message, status: 400 });
    })();
    expect(input.getAttribute('aria-invalid')).toBe('true');
    expect(host.textContent).toContain(message);
    const describedBy = input.getAttribute('aria-describedby');
    expect(describedBy).toBeTruthy();
    expect(document.getElementById(describedBy!)?.textContent).toContain(message);
  });
});
