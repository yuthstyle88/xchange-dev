// Issue #2404: Desk step 1 — disable Back while sending, and guard reserve / onSkip
// with onCleanup so leaving step 1 (go(0) / startNew) while an RPC is in flight
// never writes staffDraftId / customerId or jumps to step 2 when the RPC settles.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, Show } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../../shared/api/client';
import { setLang } from '../../../../shared/i18n';
import type { DeskStore } from '../../desk-store';
import { step } from './step';

const BACK = '← กลับไปเลือกเรท';
const SKIP = 'ข้ามข้อมูลลูกค้า →';

const PATHS = {
  OWNER_DESK: { kind: 'owner', rpc: '/api/rpc', usesDraft: false },
  STAFF_DESK: { kind: 'staff', rpc: '/api/staff/rpc', usesDraft: true },
} as const;

const JPEG = new File([new Uint8Array([0xff, 0xd8, 0xff, 1])], 'passport.jpg', { type: 'image/jpeg' });

interface DeferredRpc {
  path: string;
  action: string;
  payload: unknown;
  resolve: (body: unknown, status?: number) => void;
  rejectNetwork: () => void;
}

function deferredRpcServer() {
  const calls: DeferredRpc[] = [];
  const fetchFn = vi.fn(
    (input: RequestInfo | URL, init?: RequestInit) =>
      new Promise<Response>((resolve, reject) => {
        const body = JSON.parse(String(init?.body ?? '{}')) as { action: string; payload: unknown };
        calls.push({
          path: String(input),
          action: body.action,
          payload: body.payload,
          resolve: (resBody: unknown, status = 200) => {
            resolve(
              new Response(JSON.stringify(resBody ?? {}), {
                status,
                headers: { 'content-type': 'application/json' },
              }),
            );
          },
          rejectNetwork: () => {
            reject(new TypeError('Failed to fetch'));
          },
        });
      }),
  );
  return { api: new ApiClient(fetchFn as unknown as typeof fetch), calls };
}

function createTestStore(path: (typeof PATHS)[keyof typeof PATHS], api: ApiClient) {
  const [stepIdx, setStepIdx] = createSignal<0 | 1 | 2 | 3>(1);
  const [passport, setPassportS] = createSignal<File | undefined>();
  const [fields, setFields] = createSignal({
    name: '',
    passportNo: '',
    nationality: '',
    birthDate: '',
    passportExpiry: '',
  });
  const [identityChecked, setIC] = createSignal(false);
  const [customerId, setCID] = createSignal('');
  const [staffDraftId, setSDI] = createSignal('');
  const [entry] = createSignal<undefined>(undefined);

  const store = {
    api,
    path,
    type: () => 'BUY',
    currency: () => 'USD',
    amount: () => '100.50',
    step: stepIdx,
    config: () => ({ shiftOpen: true }),
    passport,
    setPassport: vi.fn((f: File | undefined) => setPassportS(f)),
    fields,
    setField: vi.fn((k: string, v: string, by: 'user' | 'ocr') => {
      setFields({ ...fields(), [k]: v });
      if (by === 'user') setIC(false);
    }),
    editedByUser: () => new Set<string>(),
    identityChecked,
    setIdentityChecked: vi.fn((v: boolean) => setIC(v)),
    customerId,
    setCustomerId: vi.fn((v: string) => setCID(v)),
    staffDraftId,
    setStaffDraftId: vi.fn((v: string) => setSDI(v)),
    entry,
    go: vi.fn((s: 0 | 1 | 2 | 3) => setStepIdx(s)),
    startNew: vi.fn(() => {
      setPassportS(undefined);
      setFields({ name: '', passportNo: '', nationality: '', birthDate: '', passportExpiry: '' });
      setIC(false);
      setCID('');
      setSDI('');
      setStepIdx(0);
    }),
  };
  return store;
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
  setLang('th');
});

function mountStep(store: ReturnType<typeof createTestStore>) {
  dispose = render(
    () => (
      <Show when={store.step() === 1}>
        {step.component({ store: store as unknown as DeskStore })}
      </Show>
    ),
    host,
  );
}

const q = <T extends Element>(sel: string) => host.querySelector<T>(sel);
const button = (text: string) =>
  [...host.querySelectorAll<HTMLButtonElement>('button')].find((b) => (b.textContent ?? '').includes(text));

function attach(file: File) {
  const input = q<HTMLInputElement>('input[type="file"][name="passport"]');
  expect(input, 'the passport file input').not.toBeNull();
  Object.defineProperty(input!, 'files', { value: [file], configurable: true });
  input!.dispatchEvent(new Event('change', { bubbles: true }));
}

function type(name: string, value: string) {
  const el = q<HTMLInputElement>(`input[name="${name}"]`);
  expect(el, `input ${name}`).not.toBeNull();
  el!.value = value;
  el!.dispatchEvent(new Event('input', { bubbles: true }));
  el!.dispatchEvent(new Event('change', { bubbles: true }));
}

function pickNationality(code: string) {
  const sel = q<HTMLSelectElement>('select[name="nationality"]');
  expect(sel, 'the nationality picker').not.toBeNull();
  sel!.value = code;
  sel!.dispatchEvent(new Event('change', { bubbles: true }));
}

function fillAll() {
  attach(JPEG);
  type('name', 'JOHN DOE');
  type('passportNo', 'AA1234567');
  pickNationality('USA');
  type('birthDate', '1990-01-01');
  type('passportExpiry', '2030-01-01');
}

const submit = () => q<HTMLButtonElement>('button[type="submit"]')!.click();

async function flush(): Promise<void> {
  for (let i = 0; i < 6; i++) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 10));
}

describe('steps/customer leave guard and Back disabled state (#2404)', () => {
  it('row 1: reserveCustomer in flight -> Back disabled', async () => {
    const s = deferredRpcServer();
    const store = createTestStore(PATHS.OWNER_DESK, s.api);
    mountStep(store);

    const backBtn = button(BACK)!;
    expect(backBtn).toBeDefined();
    expect(backBtn.disabled).toBe(false);

    fillAll();
    submit();

    await vi.waitFor(() => expect(s.calls).toHaveLength(1));
    expect(s.calls[0]!.action).toBe('reserveCustomer');
    expect(backBtn.disabled).toBe(true);

    // When reserveCustomer fails while still on step 1, Back re-enables
    s.calls[0]!.resolve({ error: 'เลขพาสปอร์ตไม่ถูกต้อง' }, 400);
    await flush();
    expect(backBtn.disabled).toBe(false);
  });

  it('row 2: Step left (go(0) / startNew) while reserveCustomer is in flight, then it answers -> step stays 0; customerId stays ""', async () => {
    // Case A: leave via store.go(0)
    const s1 = deferredRpcServer();
    const store1 = createTestStore(PATHS.OWNER_DESK, s1.api);
    mountStep(store1);
    fillAll();
    submit();

    await vi.waitFor(() => expect(s1.calls).toHaveLength(1));
    expect(s1.calls[0]!.action).toBe('reserveCustomer');

    store1.go(0);
    expect(store1.step()).toBe(0);

    s1.calls[0]!.resolve({ customerId: 'CX-00000042', existing: false });
    await flush();

    expect(store1.step()).toBe(0);
    expect(store1.customerId()).toBe('');
    expect(store1.setCustomerId).not.toHaveBeenCalled();
    expect(store1.go).not.toHaveBeenCalledWith(2);

    dispose?.();
    host.innerHTML = '';

    // Case B: leave via store.startNew()
    const s2 = deferredRpcServer();
    const store2 = createTestStore(PATHS.OWNER_DESK, s2.api);
    mountStep(store2);
    fillAll();
    submit();

    await vi.waitFor(() => expect(s2.calls).toHaveLength(1));
    expect(s2.calls[0]!.action).toBe('reserveCustomer');

    store2.startNew();
    expect(store2.step()).toBe(0);

    s2.calls[0]!.resolve({ customerId: 'CX-00000099', existing: false });
    await flush();

    expect(store2.step()).toBe(0);
    expect(store2.customerId()).toBe('');
    expect(store2.setCustomerId).not.toHaveBeenCalled();
    expect(store2.go).not.toHaveBeenCalledWith(2);
  });

  it('row 3: Step left while opsBeginTransaction is in flight -> staffDraftId stays ""', async () => {
    const s = deferredRpcServer();
    const store = createTestStore(PATHS.STAFF_DESK, s.api);
    mountStep(store);
    fillAll();
    submit();

    expect(s.calls).toHaveLength(1);
    expect(s.calls[0]!.action).toBe('opsBeginTransaction');

    // Leave step 1 while opsBeginTransaction is still in flight
    store.go(0);
    expect(store.step()).toBe(0);

    s.calls[0]!.resolve({ id: 'draft-stale-9' });
    await flush();

    expect(store.step()).toBe(0);
    expect(store.staffDraftId()).toBe('');
    expect(store.setStaffDraftId).not.toHaveBeenCalled();
    expect(s.calls.filter((c) => c.action === 'reserveCustomer')).toHaveLength(0);
  });

  it('edge case: A failure answer after leaving shows no error, and leaving during skip does not advance to step 2', async () => {
    // 1. Failure answer after dispose while DOM node is still inspected: no error alert rendered
    const s1 = deferredRpcServer();
    const store1 = createTestStore(PATHS.OWNER_DESK, s1.api);
    dispose = render(() => step.component({ store: store1 as unknown as DeskStore }), host);
    fillAll();
    submit();

    await vi.waitFor(() => expect(s1.calls).toHaveLength(1));
    dispose();
    dispose = undefined;

    s1.calls[0]!.resolve({ error: 'STALE-SERVER-REFUSAL' }, 400);
    await flush();

    expect(host.querySelector('[role="alert"]')).toBeNull();
    expect(host.textContent ?? '').not.toContain('STALE-SERVER-REFUSAL');
    host.innerHTML = '';

    // 2. Skip clicked, then step left while reserveCustomer is in flight: step stays 0 and customerId stays ""
    const s2 = deferredRpcServer();
    const store2 = createTestStore(PATHS.OWNER_DESK, s2.api);
    mountStep(store2);

    button(SKIP)!.click();
    await vi.waitFor(() => {
      expect(s2.calls).toHaveLength(1);
    });
    expect(s2.calls[0]!.action).toBe('reserveCustomer');

    store2.startNew();
    expect(store2.step()).toBe(0);

    s2.calls[0]!.resolve({ customerId: 'CX-SKIP-STALE', existing: false });
    await flush();

    expect(store2.step()).toBe(0);
    expect(store2.customerId()).toBe('');
    expect(store2.go).not.toHaveBeenCalledWith(2);
  });
});
