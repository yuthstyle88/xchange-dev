// Up-front tests for QW-2f (kiosk-shared-phone §5r.4a). Do not edit, delete or weaken; add tests in a file of your own.
//
// RequestActions, the request's buttons (§5r F8, R-11, W11). Rows: U4 (ยกเลิกคิว is one press, no dialog) and U5
// (the e-mail button only when emailCopy.available, with the masked address; it posts once). Only RequestActions is
// under test: the calls it makes are injected.
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from '../../../shared/api/client';
import { setAudience, setLang } from '../../../shared/i18n';
import { th } from './i18n';
import type { EmailCopy, EmailCopyAnswer, WebRequestStatus } from './queue-api';
import { makeEmailCopy } from '../../../shared/testing/fixtures/my_profile';
import { RequestActions } from './RequestActions';

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let onCancel: ReturnType<typeof vi.fn<() => Promise<void>>>;
let onDone: ReturnType<typeof vi.fn<() => Promise<void>>>;
let onEmailCopy: ReturnType<typeof vi.fn<() => Promise<EmailCopyAnswer>>>;
let confirmSpy: ReturnType<typeof vi.fn>;

const NO_EMAIL = makeEmailCopy({ available: false, to: null });
const EMAIL = makeEmailCopy({ available: true, to: 'j***@example.com' });

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
}

function q<T extends Element>(sel: string): T | null {
  return host.querySelector<T>(sel);
}

function mount(emailCopy: EmailCopy, status: WebRequestStatus | null = null): void {
  dispose = render(
    () => (
      <RequestActions
        emailCopy={emailCopy}
        status={status}
        onCancel={onCancel}
        onDone={onDone}
        onEmailCopy={onEmailCopy}
      />
    ),
    host,
  );
}

/** A promise the test settles. */
function pending<T>(): { promise: Promise<T>; resolve: (v: T) => void; reject: (e: unknown) => void } {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

beforeEach(() => {
  setAudience('customer');
  setLang('th');
  onCancel = vi.fn(async () => {});
  onDone = vi.fn(async () => {});
  onEmailCopy = vi.fn(async () => ({ sent: true }));
  confirmSpy = vi.fn(() => true);
  vi.stubGlobal('confirm', confirmSpy);
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.unstubAllGlobals();
});

describe('QW-2f: ยกเลิกคิว (U4)', () => {
  it('one press calls onCancel once, with no dialog and no confirm(); a second press in flight calls nothing', async () => {
    const call = pending<void>();
    onCancel.mockImplementation(() => call.promise);
    mount(NO_EMAIL);
    const button = q<HTMLButtonElement>('button[data-action="cancel-web-request"]');
    expect(button).not.toBeNull();
    button!.click();
    button!.click();
    await flush();
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(confirmSpy).not.toHaveBeenCalled();
    expect(document.querySelector('dialog[open]')).toBeNull();
    call.resolve();
    await flush();
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it('there is no cancel button once the status is CANCELLED', () => {
    mount(NO_EMAIL, 'CANCELLED');
    expect(q('button[data-action="done"]'), 'the buttons rendered').not.toBeNull();
    expect(q('button[data-action="cancel-web-request"]')).toBeNull();
  });
});

describe('QW-2f: เสร็จ', () => {
  it('done calls onDone once', async () => {
    mount(NO_EMAIL);
    const done = q<HTMLButtonElement>('button[data-action="done"]');
    expect(done).not.toBeNull();
    done!.click();
    await flush();
    expect(onDone).toHaveBeenCalledTimes(1);
  });
});

describe('QW-2f: ส่งเข้าอีเมล (U5)', () => {
  it('no e-mail button and no address when emailCopy.available is false', () => {
    mount(NO_EMAIL);
    expect(q('button[data-action="done"]'), 'the buttons rendered').not.toBeNull();
    expect(q('button[data-action="email-copy"]')).toBeNull();
    expect(host.textContent).not.toContain('@');
  });

  it('available: the button and the masked address beside it', () => {
    mount(EMAIL);
    expect(q('button[data-action="email-copy"]')).not.toBeNull();
    expect(host.textContent).toContain('j***@example.com');
  });

  it('a press calls onEmailCopy once even when pressed again in flight; then it says it was sent', async () => {
    const call = pending<EmailCopyAnswer>();
    onEmailCopy.mockImplementation(() => call.promise);
    mount(EMAIL);
    const button = q<HTMLButtonElement>('button[data-action="email-copy"]');
    expect(button).not.toBeNull();
    button!.click();
    button!.click();
    await flush();
    expect(onEmailCopy).toHaveBeenCalledTimes(1);
    call.resolve({ sent: true });
    await flush();
    expect(q('[role="status"]')?.textContent).toContain(th.emailSent);
  });

  it('a refused e-mail shows a worded alert', async () => {
    onEmailCopy.mockRejectedValue(new ApiError({ code: 'noVerifiedEmail', message: 'ยังไม่มีอีเมลที่ยืนยันแล้ว', status: 409 }));
    mount(EMAIL);
    const button = q<HTMLButtonElement>('button[data-action="email-copy"]');
    expect(button).not.toBeNull();
    button!.click();
    await flush();
    const alert = q('[role="alert"]');
    expect(alert, 'a worded error').not.toBeNull();
    expect(alert!.textContent?.trim()).toBeTruthy();
  });
});
