// TD-3 #1291: the one shared receipt printer (design D5, §4.4). The desk's receipt step (#543) prints through it too.
//
// The stored document is printed from a hidden frame whose sandbox allows the print dialog (`allow-modals`) and the
// parent's call into it (`allow-same-origin`), and never scripts: nothing in the document runs.

const SANDBOX = 'allow-same-origin allow-modals';

/** Prints run one at a time, so there is never more than one frame on the page. */
let queue: Promise<void> = Promise.resolve();

function printOnce(html: string): Promise<void> {
  return new Promise<void>((resolve) => {
    const frame = document.createElement('iframe');
    frame.setAttribute('sandbox', SANDBOX);
    frame.setAttribute('aria-hidden', 'true');
    frame.setAttribute('tabindex', '-1');
    frame.style.position = 'fixed';
    frame.style.right = '0';
    frame.style.bottom = '0';
    frame.style.width = '0px';
    frame.style.height = '0px';
    frame.style.border = '0';
    frame.setAttribute('srcdoc', html);
    frame.addEventListener(
      'load',
      () => {
        try {
          frame.contentWindow?.print();
        } finally {
          frame.remove();
          resolve();
        }
      },
      { once: true },
    );
    document.body.appendChild(frame);
  });
}

export const ReceiptFrame = {
  /**
   * Prints `html` through one hidden `<iframe sandbox="allow-same-origin allow-modals" srcdoc=…>` (no
   * `allow-scripts`), calls its `contentWindow.print()` once after load, then removes the frame.
   */
  print(html: string): Promise<void> {
    const next = queue.then(() => printOnce(html));
    queue = next.catch(() => undefined);
    return next;
  },
};
