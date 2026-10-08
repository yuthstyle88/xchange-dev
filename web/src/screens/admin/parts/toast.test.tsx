// V2-parts (#476): the page's one toast (ticket §7 `toast.ts`, design §12.2). Written before the
// implementation by the lane controller (claude/7ca80c). Do not edit, delete or weaken; add tests
// in a file of your own.
//
// v2 (`admin-html:22`, `:53`): one `role="status"` `aria-live="polite"` element; `toast(s)` puts the
// text in it and shows it, and hides it 5 s after the **last** call (each call restarts the
// timer). The shell renders `<Toast/>` once (§12.2).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { JSX } from 'solid-js';
import { Toast, toast } from './toast';

let dispose: (() => void) | undefined;
let root: HTMLElement;

function mount(fn: () => JSX.Element): HTMLElement {
  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(fn, root);
  return root;
}

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.runOnlyPendingTimers();
  vi.useRealTimers();
  dispose?.();
  dispose = undefined;
  root?.remove();
});

/** The text the toast shows now, or null when nothing is shown. */
function shown(el: HTMLElement): string | null {
  const status = el.querySelector('[role="status"]') as HTMLElement | null;
  if (!status || status.hidden || status.closest('[hidden]')) return null;
  const text = (status.textContent ?? '').trim();
  return text === '' ? null : text;
}

describe('toast (#476)', () => {
  it('one polite status region, showing nothing until toast is called', () => {
    const el = mount(() => <Toast />);
    const regions = el.querySelectorAll('[role="status"]');
    expect(regions.length).toBe(1);
    expect(regions[0]!.getAttribute('aria-live')).toBe('polite');
    expect(shown(el)).toBeNull();
  });

  it('shows the text for 5 seconds', () => {
    const el = mount(() => <Toast />);
    toast('เซิร์ฟเวอร์ ยืนยันการบันทึกแล้ว');
    expect(shown(el)).toBe('เซิร์ฟเวอร์ ยืนยันการบันทึกแล้ว');
    vi.advanceTimersByTime(4999);
    expect(shown(el)).toBe('เซิร์ฟเวอร์ ยืนยันการบันทึกแล้ว');
    vi.advanceTimersByTime(1);
    expect(shown(el)).toBeNull();
  });

  it('a second toast replaces the text and restarts the 5 seconds', () => {
    const el = mount(() => <Toast />);
    toast('แรก');
    vi.advanceTimersByTime(3000);
    toast('ที่สอง');
    expect(shown(el)).toBe('ที่สอง');
    vi.advanceTimersByTime(2500);
    expect(shown(el), 'the first timer does not hide the second text').toBe('ที่สอง');
    vi.advanceTimersByTime(2499);
    expect(shown(el)).toBe('ที่สอง');
    vi.advanceTimersByTime(1);
    expect(shown(el)).toBeNull();
  });
});
