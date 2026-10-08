// V1-fields (#401): TextField passes the browser's own input rules to its <input> (ticket §7.1,
// §8, rows F1 and F5). Written before the implementation by the lane controller.
// Do not edit, delete or weaken.
import { afterEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { type Form, useForm } from './form';
import { TextField } from './TextField';

let dispose: (() => void) | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
});

function mount(view: (form: Form) => ReturnType<typeof TextField>) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  let form!: Form;
  dispose = render(() => {
    form = useForm();
    return <form>{view(form)}</form>;
  }, host);
  return { host, form, inputs: Array.from(host.querySelectorAll('input')) };
}

const CONSTRAINT_ATTRS = ['required', 'pattern', 'min', 'max', 'step', 'inputmode', 'maxlength'];

describe('TextField constraint props (#401)', () => {
  it('sets required, pattern and maxlength on the input', () => {
    const { inputs } = mount((form) => (
      <TextField field={form.field('username')} label="ชื่อผู้ใช้" required pattern="[a-zA-Z0-9._\-]{3,40}" maxLength={40} />
    ));
    const input = inputs[0]!;
    expect(input.hasAttribute('required')).toBe(true);
    expect(input.required).toBe(true);
    expect(input.getAttribute('pattern')).toBe('[a-zA-Z0-9._\\-]{3,40}');
    expect(input.getAttribute('maxlength')).toBe('40');
  });

  it('sets min, max, step and inputmode on a number input', () => {
    const { inputs } = mount((form) => (
      <TextField
        field={form.field('rate')}
        label="อัตรา"
        type="number"
        required
        min="0.0001"
        max="1000000"
        step="0.0001"
        inputMode="decimal"
      />
    ));
    const input = inputs[0]!;
    expect(input.type).toBe('number');
    expect(input.getAttribute('min')).toBe('0.0001');
    expect(input.getAttribute('max')).toBe('1000000');
    expect(input.getAttribute('step')).toBe('0.0001');
    expect(input.getAttribute('inputmode')).toBe('decimal');
    expect(input.required).toBe(true);
  });

  it('absent props set nothing (F5)', () => {
    const { inputs } = mount((form) => <TextField field={form.field('username')} label="ชื่อผู้ใช้" />);
    const input = inputs[0]!;
    for (const attr of CONSTRAINT_ATTRS) {
      expect(input.hasAttribute(attr), attr).toBe(false);
    }
  });

  it('required={false} does not set required', () => {
    const { inputs } = mount((form) => <TextField field={form.field('note')} label="หมายเหตุ" required={false} />);
    expect(inputs[0]!.required).toBe(false);
  });

  it('a submit with the field empty does not call the submit function and marks and focuses it', async () => {
    const { form, inputs, host } = mount((form) => (
      <TextField field={form.field('username')} label="ชื่อผู้ใช้" required pattern="[a-zA-Z0-9._\-]{3,40}" maxLength={40} />
    ));
    const input = inputs[0]!;
    let calls = 0;
    await form.submit(async () => {
      calls += 1;
    })();
    expect(calls).toBe(0);
    expect(input.getAttribute('aria-invalid')).toBe('true');
    expect(document.activeElement).toBe(input);
    expect(form.errorFor('username')).toBe(input.validationMessage);
    const describedBy = input.getAttribute('aria-describedby');
    expect(describedBy).toBeTruthy();
    expect(host.querySelector(`#${describedBy}`)?.textContent).toContain(input.validationMessage);
  });

  it('a number field under its min stops the submit; 33.25 passes', async () => {
    const { form, inputs } = mount((form) => (
      <TextField field={form.field('rate')} label="อัตรา" type="number" required min="0.0001" max="1000000" step="0.0001" />
    ));
    const input = inputs[0]!;
    let calls = 0;
    const fn = async () => {
      calls += 1;
    };
    input.value = '0';
    await form.submit(fn)();
    expect(calls).toBe(0);
    expect(input.getAttribute('aria-invalid')).toBe('true');

    input.value = '33.25';
    await form.submit(fn)();
    expect(calls).toBe(1);
    expect(input.getAttribute('aria-invalid')).not.toBe('true');
  });

  it('a corrected field loses its mark on input', async () => {
    const { form, inputs } = mount((form) => (
      <TextField field={form.field('username')} label="ชื่อผู้ใช้" required pattern="[a-zA-Z0-9._\-]{3,40}" />
    ));
    const input = inputs[0]!;
    await form.submit(async () => undefined)();
    expect(input.getAttribute('aria-invalid')).toBe('true');

    input.value = 'cashier.01';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    expect(input.getAttribute('aria-invalid')).not.toBe('true');
    expect(input.hasAttribute('aria-describedby')).toBe(false);
  });
});
