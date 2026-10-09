import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { Field, CheckField } from './Field';

describe('Field and CheckField components (CD-0, #726)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
  });

  describe('Field', () => {
    it('has .form-label, .form-hint and the control as separate children (markup contract §2.2 & Done criteria 1)', () => {
      dispose = render(
        () => (
          <Field label="Account Number" hint="Bank account digits" unit="THB">
            <input type="text" name="account" />
          </Field>
        ),
        host,
      );

      const label = host.querySelector('label.form-field');
      expect(label).not.toBeNull();
      expect(label?.tagName).toBe('LABEL');

      // Assert that .form-label, .form-hint and the control are separate direct children
      const children = Array.from(label!.children);
      expect(children.length).toBe(3);

      // Child 0: .form-label (with unit inside)
      expect(children[0].classList.contains('form-label')).toBe(true);
      expect(children[0].querySelector('.form-unit')?.textContent).toBe('THB');

      // Child 1: .form-hint (must be a separate child, not inside form-label)
      expect(children[1].classList.contains('form-hint')).toBe(true);
      expect(children[1].textContent).toBe('Bank account digits');
      expect(children[0].querySelector('.form-hint')).toBeNull();

      // Child 2: the control input
      expect(children[2].tagName).toBe('INPUT');
    });

    it('omits .form-hint when hint is not provided', () => {
      dispose = render(
        () => (
          <Field label="Username">
            <input type="text" name="username" />
          </Field>
        ),
        host,
      );

      const label = host.querySelector('label.form-field');
      expect(label).not.toBeNull();
      expect(label?.querySelector('.form-hint')).toBeNull();

      const children = Array.from(label!.children);
      expect(children.length).toBe(2);
      expect(children[0].classList.contains('form-label')).toBe(true);
      expect(children[1].tagName).toBe('INPUT');
    });
  });

  describe('CheckField', () => {
    it('input checkbox is label.firstElementChild (markup contract §2.2 & Done criteria 1)', () => {
      const onChange = vi.fn();
      dispose = render(
        () => (
          <CheckField
            label="Enable sell"
            hint="Allow selling at this branch"
            checked={true}
            onChange={onChange}
          />
        ),
        host,
      );

      const label = host.querySelector('label.check-field');
      expect(label).not.toBeNull();
      expect(label?.tagName).toBe('LABEL');

      // Strict Done Criteria check: input is label.firstElementChild
      const firstChild = label!.firstElementChild;
      expect(firstChild).not.toBeNull();
      expect(firstChild?.tagName).toBe('INPUT');
      expect((firstChild as HTMLInputElement).type).toBe('checkbox');
      expect((firstChild as HTMLInputElement).checked).toBe(true);

      // The text container sits beside the checkbox
      const secondChild = label!.children[1];
      expect(secondChild.classList.contains('check-text')).toBe(true);
      expect(secondChild.querySelector('.form-label')?.textContent).toBe('Enable sell');
      expect(secondChild.querySelector('.form-hint')?.textContent).toBe('Allow selling at this branch');
    });

    it('passes through disabled, required, name, and responds to change events', () => {
      let isChecked = false;
      const onChange = vi.fn((val: boolean) => {
        isChecked = val;
      });

      dispose = render(
        () => (
          <CheckField
            id="test-check"
            name="enabled"
            label="Verified"
            checked={isChecked}
            disabled={true}
            required={true}
            onChange={onChange}
          />
        ),
        host,
      );

      const input = host.querySelector<HTMLInputElement>('input[type="checkbox"]');
      expect(input).not.toBeNull();
      expect(input?.id).toBe('test-check');
      expect(input?.name).toBe('enabled');
      expect(input?.disabled).toBe(true);
      expect(input?.required).toBe(true);
      expect(input?.checked).toBe(false);

      // Trigger change
      input!.checked = true;
      input!.dispatchEvent(new Event('change'));
      expect(onChange).toHaveBeenCalledWith(true);
    });
  });
});
