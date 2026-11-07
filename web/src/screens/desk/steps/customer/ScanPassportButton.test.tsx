// XC Counter CA-5 (docs/design/xc-counter-app.md §3, §4): the desk's "สแกนพาสปอร์ต" button.
// Written before the implementation. Do not edit, delete or weaken; add tests in a file of your own.
//
// | # | Case | Required |
// |---|---|---|
// | S1 | no `window.xcCounter` | no button |
// | S2 | `capabilities.scanner` false | no button |
// | S3 | `info()` rejects | no button, nothing thrown |
// | S4 | `capabilities.scanner` true | the button, enabled |
// | S5 | click, scan pending | `scanPassport` once; disabled; reads "กำลังสแกน…"; a second click does not scan again |
// | S6 | `{ok:true}` | `setPassport` gets a File `passport-scan.jpg`, `image/jpeg`, with the blob's bytes; the button is back |
// | S7 | each of the 5 refusals | its Thai message in `role="alert"`; `setPassport` not called; the button is back |
// | S8 | the next scan after a refusal | the old message is gone |
// | S9 | `scanPassport` rejects | the `device` message; `setPassport` not called |
// | S10 | a blob over 15 MB, or empty | `fileRefusal`'s message; `setPassport` not called |
// | S11 | in the step, every outcome | the button sits in the step, and the file field stays enabled and still takes a file |
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../../shared/api/client';
import { setLang } from '../../../../shared/i18n';
import type { CounterInfo, ScanRefusal, ScanResult, XcCounter } from '../../../../shared/xc-counter/bridge';
import type { DeskStore } from '../../desk-store';
import { ScanPassportButton } from './ScanPassportButton';
import { fileRefusal, step } from './step';

const SCAN = 'สแกนพาสปอร์ต';
const SCANNING = 'กำลังสแกน…';
const MAX_FILE_BYTES = 15_728_640;

const REFUSALS: Array<[ScanRefusal, string]> = [
  ['noScanner', 'ไม่พบเครื่องสแกน'],
  ['busy', 'เครื่องสแกนกำลังทำงานอยู่'],
  ['timeout', 'สแกนไม่สำเร็จ หมดเวลา'],
  ['coverOpen', 'ฝาเครื่องสแกนเปิดอยู่'],
  ['device', 'เครื่องสแกนขัดข้อง'],
];

function info(scanner: boolean): CounterInfo {
  return {
    agentId: 'agent-1',
    counterName: 'เคาน์เตอร์ 1',
    branchId: 'XC-B01',
    capabilities: { cashDrawer: false, scanner, kioskLock: true },
    appVersion: '1.0.0',
    build: 'abc1234',
  };
}

function bridge(over: Partial<XcCounter> = {}): XcCounter {
  const counter: XcCounter = {
    info: vi.fn(async () => info(true)),
    scanPassport: vi.fn(async (): Promise<ScanResult> => ({ ok: false, reason: 'device' })),
    bindingNonce: vi.fn(async () => 'v1.nonce'),
    ...over,
  };
  window.xcCounter = counter;
  return counter;
}

function deferred<T>() {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

/** The whole store of the customer step, as its neighbours build it, for S11. */
function stepStore() {
  const [passport, setPassportS] = createSignal<File | undefined>();
  const [fields, setFields] = createSignal({ name: '', passportNo: '', nationality: '', birthDate: '', passportExpiry: '' });
  const setPassport = vi.fn((f: File | undefined) => setPassportS(f));
  return {
    api: new ApiClient(vi.fn() as never),
    path: { kind: 'staff', rpc: '/api/staff/rpc', usesDraft: true },
    branch: () => 'XC-B01',
    type: () => 'BUY',
    currency: () => 'USD',
    amount: () => '100.50',
    step: () => 1,
    config: () => ({ shiftOpen: true }),
    passport,
    setPassport,
    fields,
    setField: vi.fn((k: string, v: string) => setFields({ ...fields(), [k]: v })),
    editedByUser: () => new Set<string>(),
    identityChecked: () => false,
    setIdentityChecked: vi.fn(),
    customerId: () => '',
    setCustomerId: vi.fn(),
    meId: () => '',
    heldBooking: () => undefined,
    setHeldBooking: vi.fn(),
    staffDraftId: () => '',
    setStaffDraftId: vi.fn(),
    entry: () => undefined,
    go: vi.fn(),
  };
}

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
  delete window.xcCounter;
  setLang('th');
});

async function settle() {
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}

function mount() {
  const setPassport = vi.fn();
  const store = { setPassport } as unknown as DeskStore;
  dispose = render(() => <ScanPassportButton store={store} />, host);
  return { setPassport };
}

function mountStep() {
  const store = stepStore();
  dispose = render(() => step.component({ store: store as unknown as DeskStore }), host);
  return store;
}

const buttonNamed = (text: string) =>
  [...host.querySelectorAll<HTMLButtonElement>('button')].find((b) => (b.textContent ?? '').trim() === text);

function scanButton(): HTMLButtonElement {
  const b = buttonNamed(SCAN);
  expect(b, `the "${SCAN}" button`).toBeDefined();
  return b!;
}

function alertText(): string {
  return [...host.querySelectorAll('[role="alert"]')].map((a) => a.textContent ?? '').join(' ');
}

describe('ScanPassportButton: when it shows', () => {
  it('S1: absent without xcCounter', async () => {
    mount();
    await settle();
    expect(buttonNamed(SCAN)).toBeUndefined();
  });

  it('S2: absent when capabilities.scanner is false', async () => {
    bridge({ info: vi.fn(async () => info(false)) });
    mount();
    await settle();
    expect(buttonNamed(SCAN)).toBeUndefined();
  });

  it('S3: absent when info() rejects', async () => {
    const counter = bridge({ info: vi.fn(async () => Promise.reject(new Error('no server'))) });
    mount();
    await settle();
    expect(counter.info).toHaveBeenCalled();
    expect(buttonNamed(SCAN)).toBeUndefined();
  });

  it('S4: shown and enabled when capabilities.scanner is true', async () => {
    bridge();
    mount();
    await settle();
    expect(scanButton().disabled).toBe(false);
    expect(scanButton().getAttribute('type'), 'never submits the step form').toBe('button');
  });
});

describe('ScanPassportButton: a scan', () => {
  it('S5: scans once and is disabled with "กำลังสแกน…" while pending', async () => {
    const pending = deferred<ScanResult>();
    const counter = bridge({ scanPassport: vi.fn(() => pending.promise) });
    mount();
    await settle();
    const button = scanButton();
    button.click();
    await settle();
    const busy = buttonNamed(SCANNING);
    expect(busy, `the button reads "${SCANNING}"`).toBeDefined();
    expect(busy!.disabled).toBe(true);
    busy!.click();
    await settle();
    expect(counter.scanPassport).toHaveBeenCalledTimes(1);
    pending.resolve({ ok: false, reason: 'busy' });
    await settle();
  });

  it('S6: ok hands setPassport a File named passport-scan.jpg, image/jpeg, with the blob bytes', async () => {
    const bytes = new Uint8Array([0xff, 0xd8, 0xff, 7, 8, 9]);
    bridge({ scanPassport: vi.fn(async (): Promise<ScanResult> => ({ ok: true, file: new Blob([bytes], { type: 'image/jpeg' }) })) });
    const { setPassport } = mount();
    await settle();
    scanButton().click();
    await settle();
    expect(setPassport).toHaveBeenCalledTimes(1);
    const file = setPassport.mock.calls[0]![0] as File;
    expect(file).toBeInstanceOf(File);
    expect(file.name).toBe('passport-scan.jpg');
    expect(file.type).toBe('image/jpeg');
    expect(new Uint8Array(await file.arrayBuffer())).toEqual(bytes);
    expect(scanButton().disabled, 'the button is back').toBe(false);
    expect(alertText()).toBe('');
  });

  it.each(REFUSALS)('S7: the refusal %s shows its Thai message and sets nothing', async (reason, message) => {
    bridge({ scanPassport: vi.fn(async (): Promise<ScanResult> => ({ ok: false, reason })) });
    const { setPassport } = mount();
    await settle();
    scanButton().click();
    await settle();
    expect(alertText()).toContain(message);
    expect(setPassport).not.toHaveBeenCalled();
    expect(scanButton().disabled, 'the button is back').toBe(false);
  });

  it('S8: the next scan clears the old message', async () => {
    const pending = deferred<ScanResult>();
    const scanPassport = vi
      .fn<() => Promise<ScanResult>>()
      .mockResolvedValueOnce({ ok: false, reason: 'coverOpen' })
      .mockReturnValueOnce(pending.promise);
    bridge({ scanPassport });
    mount();
    await settle();
    scanButton().click();
    await settle();
    expect(alertText()).toContain('ฝาเครื่องสแกนเปิดอยู่');
    scanButton().click();
    await settle();
    expect(alertText()).not.toContain('ฝาเครื่องสแกนเปิดอยู่');
    pending.resolve({ ok: false, reason: 'busy' });
    await settle();
  });

  it('S9: a scanPassport that rejects shows the device message and sets nothing', async () => {
    bridge({ scanPassport: vi.fn(async () => Promise.reject(new Error('ipc'))) });
    const { setPassport } = mount();
    await settle();
    scanButton().click();
    await settle();
    expect(alertText()).toContain('เครื่องสแกนขัดข้อง');
    expect(setPassport).not.toHaveBeenCalled();
    expect(scanButton().disabled).toBe(false);
  });

  it.each([
    ['over the size limit', new Uint8Array(MAX_FILE_BYTES + 1)],
    ['empty', new Uint8Array(0)],
  ])('S10: a blob %s shows fileRefusal\'s message and sets nothing', async (_name, bytes) => {
    bridge({ scanPassport: vi.fn(async (): Promise<ScanResult> => ({ ok: true, file: new Blob([bytes], { type: 'image/jpeg' }) })) });
    const { setPassport } = mount();
    await settle();
    scanButton().click();
    await settle();
    const refusal = fileRefusal(new File([bytes], 'passport-scan.jpg', { type: 'image/jpeg' }));
    expect(refusal, 'fileRefusal refuses this size').toBeTypeOf('string');
    expect(alertText()).toContain(refusal);
    expect(setPassport).not.toHaveBeenCalled();
  });
});

describe('ScanPassportButton in the customer step', () => {
  function expectFileFieldUsable(store: ReturnType<typeof stepStore>) {
    const input = host.querySelector<HTMLInputElement>('input[type="file"][name="passport"]');
    expect(input, 'the passport file field').not.toBeNull();
    expect(input!.disabled).toBe(false);
    const file = new File([new Uint8Array([0xff, 0xd8, 0xff, 1])], 'attached.jpg', { type: 'image/jpeg' });
    Object.defineProperty(input!, 'files', { value: [file], configurable: true });
    input!.dispatchEvent(new Event('change', { bubbles: true }));
    expect(store.setPassport).toHaveBeenLastCalledWith(file);
  }

  it('S11: the file field still takes a file while pending', async () => {
    const pending = deferred<ScanResult>();
    bridge({ scanPassport: vi.fn(() => pending.promise) });
    const store = mountStep();
    await settle();
    scanButton().click();
    await settle();
    expectFileFieldUsable(store);
    pending.resolve({ ok: false, reason: 'busy' });
    await settle();
  });

  it.each(REFUSALS)('S11: the file field still takes a file after the refusal %s', async (reason) => {
    bridge({ scanPassport: vi.fn(async (): Promise<ScanResult> => ({ ok: false, reason })) });
    const store = mountStep();
    await settle();
    scanButton().click();
    await settle();
    expectFileFieldUsable(store);
  });

  it('S11: the file field still takes a file after a good scan', async () => {
    bridge({
      scanPassport: vi.fn(async (): Promise<ScanResult> => ({ ok: true, file: new Blob([new Uint8Array([1])], { type: 'image/jpeg' }) })),
    });
    const store = mountStep();
    await settle();
    scanButton().click();
    await settle();
    expect((store.passport() as File | undefined)?.name).toBe('passport-scan.jpg');
    expectFileFieldUsable(store);
  });

  it('S11: a plain browser keeps the file field and shows no button', async () => {
    const store = mountStep();
    await settle();
    expect(buttonNamed(SCAN)).toBeUndefined();
    expectFileFieldUsable(store);
  });
});
