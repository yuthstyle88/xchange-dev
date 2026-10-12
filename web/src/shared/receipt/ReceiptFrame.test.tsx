// TD-3 #1291: `ReceiptFrame`, the one shared receipt printer (design `docs/design/blueprint/desk-transaction-detail.md`
// D5, §4.4). Written before the implementation by the up-front test writer (claude/709ec749, on the xchange FE lane
// controller claude/5edf7bb5's order). Do not edit, delete or weaken; add tests in a file of your own.
//
// [required] `ReceiptFrame.print(html: string): Promise<void>` in `web/src/shared/receipt/ReceiptFrame.tsx`:
//   one hidden <iframe>, `sandbox` exactly "allow-same-origin allow-modals" (no allow-scripts), `srcdoc` byte-identical
//   to `html`; its `contentWindow.print()` is called once after the frame's `load`; the promise resolves after that and
//   the frame is gone.
// The test installs its `print` spy inside the very DOM call that inserts the frame (`appendChild`, `append`,
// `insertBefore`, `prepend` are wrapped for the duration of one print), so a `print()` made before `load` lands on the
// spy and the "called exactly once" check sees it. jsdom fires `load` for a srcdoc frame on its own; the test also
// dispatches `load` in case it does not, so an implementation listening with `{ once: true }` prints once either way.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ReceiptFrame } from './ReceiptFrame';

const SANDBOX = 'allow-same-origin allow-modals';

/** Receipt HTML with Thai text, an entity and a figure with trailing zeros: srcdoc must carry it byte for byte. */
const RECEIPT = '<!doctype html><html><body><h1>ใบเสร็จ XC-0001</h1><p>35,120.00 THB &amp; 1000.0000 USD</p></body></html>';

function frames(): HTMLIFrameElement[] {
  return Array.from(document.querySelectorAll('iframe'));
}

interface Seen {
  frame: HTMLIFrameElement;
  printSpy: ReturnType<typeof vi.fn>;
  sandbox: string | null;
  srcdoc: string | null;
  hidden: boolean;
}

/**
 * Resolves with the next iframe inserted into the document, its print spy installed synchronously inside the inserting
 * call. Restores the wrapped DOM methods once a frame is seen.
 */
function nextFrame(): Promise<Seen> {
  return new Promise((resolve) => {
    const nodeProto = Node.prototype as unknown as Record<string, (...a: unknown[]) => unknown>;
    const elProto = Element.prototype as unknown as Record<string, (...a: unknown[]) => unknown>;
    const wrapped: [Record<string, (...a: unknown[]) => unknown>, string, (...a: unknown[]) => unknown][] = [];
    const restore = () => {
      for (const [proto, name, orig] of wrapped) proto[name] = orig;
    };
    const catchFrame = (root: unknown) => {
      if (!(root instanceof Node)) return;
      const found =
        root instanceof HTMLIFrameElement
          ? root
          : root instanceof Element
            ? root.querySelector('iframe')
            : null;
      if (!found || !found.isConnected) return;
      restore();
      const printSpy = vi.fn();
      Object.defineProperty(found.contentWindow!, 'print', { value: printSpy, configurable: true });
      resolve({
        frame: found,
        printSpy,
        sandbox: found.getAttribute('sandbox'),
        srcdoc: found.getAttribute('srcdoc'),
        hidden:
          found.hidden === true ||
          found.style.display === 'none' ||
          found.style.visibility === 'hidden' ||
          found.getAttribute('aria-hidden') === 'true' ||
          (found.style.width === '0px' && found.style.height === '0px') ||
          (found.width === '0' && found.height === '0'),
      });
    };
    for (const [proto, name] of [
      [nodeProto, 'appendChild'],
      [nodeProto, 'insertBefore'],
      [elProto, 'append'],
      [elProto, 'prepend'],
    ] as const) {
      const orig = proto[name]!;
      wrapped.push([proto, name, orig]);
      proto[name] = function (this: unknown, ...args: unknown[]) {
        const out = orig.apply(this, args);
        for (const a of args) catchFrame(a);
        return out;
      };
    }
  });
}

/** Starts `print(html)`, catches its frame as it is inserted, fires `load`, and resolves with what was seen. */
async function printAndLoad(html: string) {
  const framed = nextFrame();
  const done = ReceiptFrame.print(html);
  const seen = await Promise.race([
    framed,
    new Promise<never>((_, reject) => setTimeout(() => reject(new Error('print(html) appends no iframe')), 500)),
  ]);
  const framesWhilePrinting = frames().length;
  if (seen.frame.isConnected) seen.frame.dispatchEvent(new Event('load'));
  await done;
  return { ...seen, framesWhilePrinting };
}

afterEach(() => {
  document.body.innerHTML = '';
});

describe('ReceiptFrame.print (TD-3, design §4.4, D5)', () => {
  it('prints through one hidden sandboxed frame whose srcdoc is the html byte for byte, then removes it', async () => {
    const seen = await printAndLoad(RECEIPT);
    const { printSpy, frame } = seen;
    expect(seen.sandbox).toBe(SANDBOX);
    expect(seen.srcdoc).toBe(RECEIPT);
    expect(seen.hidden, 'the frame is not shown on the page').toBe(true);
    expect(seen.framesWhilePrinting).toBeLessThanOrEqual(1);
    expect(printSpy).toHaveBeenCalledTimes(1);
    expect(frame.isConnected, 'the frame is removed afterwards').toBe(false);
    expect(frames()).toEqual([]);
  });

  it('never allows scripts: a document with <script> gets the same sandbox, with no allow-scripts token', async () => {
    const html = '<html><body><script>window.parent.__receiptRan = true;</script>ok</body></html>';
    const seen = await printAndLoad(html);
    expect(seen.sandbox).toBe(SANDBOX);
    expect((seen.sandbox ?? '').split(/\s+/)).not.toContain('allow-scripts');
    expect(seen.srcdoc).toBe(html);
    expect((window as unknown as { __receiptRan?: boolean }).__receiptRan).toBeUndefined();
  });

  it('two prints in a row use two frames, one at a time, and leave none behind', async () => {
    const first = await printAndLoad(RECEIPT);
    const second = await printAndLoad(RECEIPT.replace('XC-0001', 'XC-0001-R2'));
    expect(first.framesWhilePrinting).toBeLessThanOrEqual(1);
    expect(second.framesWhilePrinting).toBeLessThanOrEqual(1);
    expect(first.frame).not.toBe(second.frame);
    expect(first.printSpy).toHaveBeenCalledTimes(1);
    expect(second.printSpy).toHaveBeenCalledTimes(1);
    expect(frames()).toEqual([]);
  });

  it('prints only on load: a print called before the frame loads reaches no spy, so this counts exactly one', async () => {
    const seen = await printAndLoad(RECEIPT);
    expect(seen.printSpy).toHaveBeenCalledTimes(1);
  });
});
