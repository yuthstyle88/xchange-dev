import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th } from '../i18n';
import { ReasonDialog } from './ReasonDialog';

function typeInto(input: HTMLInputElement, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
}

function buttons(root: ParentNode): HTMLButtonElement[] {
  return Array.from(root.querySelectorAll('button'));
}

describe('ReasonDialog reset tests (#2248: reason must not carry over to the next open)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.restoreAllMocks();
  });

  function mount(onConfirm = vi.fn()) {
    const [open, setOpen] = createSignal(false);
    const onCancel = vi.fn();
    dispose = render(
      () => (
        <ReasonDialog
          open={open()}
          title="Release hold"
          onConfirm={(r) => {
            onConfirm(r);
            setOpen(false);
          }}
          onCancel={() => {
            onCancel();
            setOpen(false);
          }}
        />
      ),
      host,
    );
    return { setOpen, onConfirm, onCancel };
  }

  const input = (): HTMLInputElement => host.querySelector('input') as HTMLInputElement;

  it('row 1: open, type "broken note", confirm, open again -> the input is empty', () => {
    const { setOpen } = mount();
    setOpen(true);
    typeInto(input(), 'broken note');
    buttons(host)[1].click();
    expect(host.querySelector('dialog')).toBeNull();

    setOpen(true);
    expect(input().value).toBe('');
  });

  it('row 1b: confirming right after reopening does not send the previous reason', () => {
    const { setOpen, onConfirm } = mount();
    setOpen(true);
    typeInto(input(), 'broken note');
    buttons(host)[1].click();

    setOpen(true);
    buttons(host)[1].click();
    expect(onConfirm).toHaveBeenLastCalledWith('');
  });

  it('row 2: open, type, cancel, open again -> the input is empty', () => {
    const { setOpen, onCancel } = mount();
    setOpen(true);
    typeInto(input(), 'typed then cancelled');
    buttons(host)[0].click();
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(host.querySelector('dialog')).toBeNull();

    setOpen(true);
    expect(input().value).toBe('');
  });

  it('row 3 (kept): open, type "x", confirm -> onConfirm receives "x"', () => {
    const { setOpen, onConfirm } = mount();
    setOpen(true);
    typeInto(input(), 'x');
    buttons(host)[1].click();
    expect(onConfirm).toHaveBeenCalledTimes(1);
    expect(onConfirm).toHaveBeenCalledWith('x');
  });

  it('row 4 (kept): placeholder and labels unchanged', () => {
    const { setOpen } = mount();
    setOpen(true);
    expect(host.querySelector('h3')?.textContent).toBe('Release hold');
    expect(input().placeholder).toBe(th.reasonPlaceholder);
    expect(buttons(host).map((b) => b.textContent)).toEqual([th.cancel, th.confirm]);
  });

  it('edge: two dialogs on one card share no state; each resets on its own open', () => {
    const [openA, setOpenA] = createSignal(false);
    const [openB, setOpenB] = createSignal(false);
    const confirmA = vi.fn();
    const confirmB = vi.fn();
    const a = document.createElement('div');
    const b = document.createElement('div');
    host.append(a, b);
    const disposeA = render(
      () => (
        <ReasonDialog
          open={openA()}
          title="A"
          onConfirm={(r) => {
            confirmA(r);
            setOpenA(false);
          }}
          onCancel={() => setOpenA(false)}
        />
      ),
      a,
    );
    const disposeB = render(
      () => (
        <ReasonDialog
          open={openB()}
          title="B"
          onConfirm={(r) => {
            confirmB(r);
            setOpenB(false);
          }}
          onCancel={() => setOpenB(false)}
        />
      ),
      b,
    );
    dispose = () => {
      disposeA();
      disposeB();
    };
    const inA = (): HTMLInputElement => a.querySelector('input') as HTMLInputElement;
    const inB = (): HTMLInputElement => b.querySelector('input') as HTMLInputElement;

    setOpenA(true);
    setOpenB(true);
    typeInto(inA(), 'reason A');
    expect(inB().value).toBe('');
    typeInto(inB(), 'reason B');
    buttons(a)[1].click();
    expect(confirmA).toHaveBeenCalledWith('reason A');
    // B is still open and keeps its own text.
    expect(inB().value).toBe('reason B');
    buttons(b)[1].click();
    expect(confirmB).toHaveBeenCalledWith('reason B');

    setOpenA(true);
    expect(inA().value).toBe('');
    setOpenB(true);
    expect(inB().value).toBe('');
  });
});
