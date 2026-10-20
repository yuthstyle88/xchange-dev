// RB-8 up-front tests (feat/fe-rateboard skeleton; rate-display.md §9.2, §9.6): the split-flap cell. One span per
// character, only changed characters flip, an arrow and glow when `direction` is not 0. The implementer makes these
// pass and must not edit them.
import { afterEach, describe, expect, it } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { FlapPrice } from './FlapPrice';

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function mount(initial: string) {
  const [text, setText] = createSignal(initial);
  const [direction, setDirection] = createSignal<-1 | 0 | 1>(0);
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <FlapPrice text={text()} direction={direction()} />, host);
  return {
    set(next: string, dir: -1 | 0 | 1) {
      setDirection(dir);
      setText(next);
    },
  };
}

const chars = () => Array.from(host.querySelectorAll<HTMLElement>('.xc-rb-ch'));
const flipped = () => chars().filter((c) => c.classList.contains('xc-rb-flip')).map((c) => c.textContent);
const root = () => host.querySelector<HTMLElement>('.xc-rb-price')!;
const arrow = () => host.querySelector('em')?.textContent ?? null;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
});

describe('FlapPrice (RB-8)', () => {
  it('first render: one span per character, separators marked, nothing flips, no arrow', () => {
    mount('1,035.42');
    expect(chars().map((c) => c.textContent)).toEqual(['1', ',', '0', '3', '5', '.', '4', '2']);
    expect(chars().filter((c) => c.classList.contains('xc-rb-sep')).map((c) => c.textContent)).toEqual([',', '.']);
    expect(flipped()).toEqual([]);
    expect(arrow()).toBeNull();
  });

  it('a rise: only the changed characters flip, an up arrow, the up class', () => {
    const c = mount('35.42');
    c.set('35.45', 1);
    expect(chars().map((x) => x.textContent).join('')).toBe('35.45');
    expect(flipped()).toEqual(['5']);
    expect(arrow()).toBe('▲');
    expect(root().classList.contains('xc-rb-up')).toBe(true);
    expect(root().classList.contains('xc-rb-down')).toBe(false);
  });

  it('a fall: a down arrow and the down class, and the up class goes', () => {
    const c = mount('35.45');
    c.set('35.46', 1);
    c.set('35.40', -1);
    expect(arrow()).toBe('▼');
    expect(root().classList.contains('xc-rb-down')).toBe(true);
    expect(root().classList.contains('xc-rb-up')).toBe(false);
  });

  it('characters are compared from the right when the length changes', () => {
    const c = mount('9.999');
    c.set('10.00', 1);
    expect(chars().map((x) => x.textContent).join('')).toBe('10.00');
    expect(flipped()).toEqual(['1', '0', '.', '0', '0']);
  });

  it('a change with direction 0 flips the changed characters but shows no arrow', () => {
    const c = mount('—');
    c.set('35.42', 0);
    expect(arrow()).toBeNull();
    expect(chars().map((x) => x.textContent).join('')).toBe('35.42');
  });

  it('the same text again flips nothing', () => {
    const c = mount('35.42');
    c.set('35.42', 0);
    expect(flipped()).toEqual([]);
  });
});
