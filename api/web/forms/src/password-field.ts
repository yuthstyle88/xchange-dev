// PasswordField: one password input with its show/hide eye inside the right edge (#378).
//
// Every password input on a served page gets the same component:
//
//   <span class="xc-password">            ← takes the input's place in the page
//     <input type="password" data-xc-revealable>
//     <button class="xc-reveal" data-xc-reveal>  ← absolutely positioned, inside the field
//   </span>
//
// The eye stays the input's next sibling: the submit handler and the orphan sweep in
// install.ts find it that way. The component never reads or writes the field's value.

export const WRAPPER_CLASS = 'xc-password';
export const REVEAL_CLASS = 'xc-reveal';
export const REVEALABLE_ATTR = 'data-xc-revealable';
export const REVEAL_ATTR = 'data-xc-reveal';

/** What the eye will do when pressed, in Thai. */
export function revealLabel(shown: boolean): string {
  return shown ? 'ซ่อนรหัสผ่าน' : 'แสดงรหัสผ่าน';
}

/** Turns the field to text or back to a password, keeping focus and the caret. Answers "now shown". */
export function flipReveal(input: HTMLInputElement | null | undefined): boolean {
  if (!input) return false;
  const start = input.selectionStart;
  const end = input.selectionEnd;
  const shown = input.type === 'password';
  input.type = shown ? 'text' : 'password';
  if (typeof input.setSelectionRange === 'function' && start != null && end != null) {
    try {
      input.setSelectionRange(start, end);
    } catch {
      // a field type without a selection
    }
  }
  if (typeof input.focus === 'function') {
    try {
      input.focus();
    } catch {
      // a detached field
    }
  }
  return shown;
}

/** Sets the eye's label and pressed state to match the field. */
export function syncReveal(button: Element, shown: boolean): void {
  button.setAttribute('aria-label', revealLabel(shown));
  button.setAttribute('aria-pressed', String(shown));
}

const SVG_NS = 'http://www.w3.org/2000/svg';

function svg(doc: Document, tag: string, attrs: Record<string, string>): Element {
  const el = doc.createElementNS(SVG_NS, tag);
  for (const [name, value] of Object.entries(attrs)) el.setAttribute(name, value);
  return el;
}

/** The eye: a plain `type="button"` that reveals `input` and never submits the form. */
export function makeRevealButton(doc: Document, input: HTMLInputElement): HTMLButtonElement {
  const button = doc.createElement('button');
  button.type = 'button';
  button.setAttribute('type', 'button');
  button.classList.add(REVEAL_CLASS);
  syncReveal(button, false);

  const icon = svg(doc, 'svg', {
    viewBox: '0 0 24 24',
    width: '18',
    height: '18',
    fill: 'none',
    stroke: 'currentColor',
    'stroke-width': '2',
    'stroke-linecap': 'round',
    'stroke-linejoin': 'round',
    'aria-hidden': 'true',
  });
  icon.appendChild(svg(doc, 'path', { d: 'M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z' }));
  icon.appendChild(svg(doc, 'circle', { cx: '12', cy: '12', r: '3' }));
  button.appendChild(icon);

  button.addEventListener('click', (e) => {
    e.preventDefault();
    syncReveal(button, flipReveal(input));
  });
  return button;
}

/** Turns every revealed field back into a password field. */
export function concealAll(inputs: ArrayLike<HTMLInputElement> | null | undefined): void {
  if (!inputs) return;
  for (let i = 0; i < inputs.length; i++) {
    const input = inputs[i];
    if (input && input.type === 'text') input.type = 'password';
  }
}

function isWrapper(node: ParentNode | null): node is HTMLElement {
  const el = node as HTMLElement | null;
  return !!el && !!el.classList && typeof el.classList.contains === 'function' && el.classList.contains(WRAPPER_CLASS);
}

/**
 * Moves the field's vertical margins onto the wrapper, so the eye — pinned to the wrapper's
 * top and bottom — is centred on the field. Pages give inputs a margin (the kept staff.css:
 * `margin-top:8px`); left on the input it would sit inside the wrapper and push the field down.
 */
function carryMargins(doc: Document, input: HTMLInputElement, wrapper: HTMLElement): void {
  const view = doc.defaultView;
  if (!view || typeof view.getComputedStyle !== 'function') return;
  const style = view.getComputedStyle(input);
  wrapper.style.marginTop = style.marginTop;
  wrapper.style.marginBottom = style.marginBottom;
  input.style.marginTop = '0';
  input.style.marginBottom = '0';
}

/**
 * Wraps `input` in the PasswordField component, in place. Answers the wrapper, the existing
 * wrapper when the field is already wrapped, or null for a field that is not in a tree.
 */
export function wrapPasswordField(doc: Document, input: HTMLInputElement): HTMLElement | null {
  const parent = input.parentNode;
  if (!parent) return null;
  if (isWrapper(parent)) return parent;

  const wrapper = doc.createElement('span');
  wrapper.classList.add(WRAPPER_CLASS);
  parent.insertBefore(wrapper, input);
  wrapper.appendChild(input);
  carryMargins(doc, input, wrapper);

  input.setAttribute(REVEALABLE_ATTR, 'true');
  const button = makeRevealButton(doc, input);
  button.setAttribute(REVEAL_ATTR, 'true');
  wrapper.appendChild(button);
  return wrapper;
}
