import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../../shared/i18n';
import { ApiClient } from '../../../../shared/api/client';
import { STAFF_DESK, type DeskPath } from '../../desk-path';
import type { DeskEntryState, DeskStore, PassportFields } from '../../desk-store';
import { step } from './step';

const SKIP = 'ข้ามข้อมูลลูกค้า →';

const JPEG = new File([new Uint8Array([0xff, 0xd8, 0xff, 1])], 'passport.jpg', { type: 'image/jpeg' });

type RpcCall = { path: string; action: string; payload: Record<string, unknown>; at: number };

function rpcServer(
  answer: (action: string, payload: Record<string, unknown>) => { status: number; body?: unknown },
): { api: ApiClient; calls: RpcCall[] } {
  const calls: RpcCall[] = [];
  let clock = 0;
  const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const body = JSON.parse(String(init?.body ?? '{}')) as {
      action: string;
      payload: Record<string, unknown>;
    };
    // #3008: the staff draft is v3 draftBegin (POST /api/tx-drafts), not the retired opsBeginTransaction RPC.
    const isDraft = String(input) === '/api/tx-drafts';
    const call: RpcCall = {
      path: String(input),
      action: isDraft ? 'draftBegin' : body.action,
      payload: isDraft ? (body as unknown as Record<string, unknown>) : body.payload,
      at: clock++,
    };
    calls.push(call);
    const a = answer(call.action, call.payload);
    return new Response(JSON.stringify(a.body ?? {}), {
      status: a.status,
      headers: { 'content-type': 'application/json' },
    });
  });
  return { api: new ApiClient(fetchFn as unknown as typeof fetch), calls };
}

const OK = (action: string): { status: number; body: unknown } =>
  action === 'draftBegin'
    ? { status: 200, body: { id: 'draft-9' } }
    : { status: 200, body: { customerId: 'CX-00000042', existing: false } };

function stubStore(path: DeskPath, api: ApiClient): DeskStore {
  const [passport, setPassportS] = createSignal<File | undefined>();
  const [fields, setFields] = createSignal<PassportFields>({
    name: '',
    passportNo: '',
    nationality: '',
    birthDate: '',
    passportExpiry: '',
  });
  const [identityChecked, setIC] = createSignal(false);
  const [customerId, setCID] = createSignal('');
  const [staffDraftId, setSDI] = createSignal('');
  const [entry] = createSignal<DeskEntryState | undefined>(undefined);
  const store = {
    api,
    path,
    branch: () => 'XC-B01',
    type: () => 'BUY' as const,
    currency: () => 'USD',
    amount: () => '100.50',
    step: () => 1 as const,
    config: () => undefined,
    passport,
    setPassport: vi.fn((f: File | undefined) => setPassportS(f)),
    fields,
    setField: vi.fn((k: keyof PassportFields, v: string, by: 'user' | 'ocr') => {
      setFields({ ...fields(), [k]: v });
      if (by === 'user') setIC(false);
    }),
    editedByUser: () => new Set<keyof PassportFields>(),
    identityChecked,
    setIdentityChecked: vi.fn((v: boolean) => setIC(v)),
    customerId,
    setCustomerId: vi.fn((v: string) => setCID(v)),
    staffDraftId,
    setStaffDraftId: vi.fn((v: string) => setSDI(v)),
    entry,
    go: vi.fn(),
  };
  return store as unknown as DeskStore;
}

function stubCanvasContext(): CanvasRenderingContext2D {
  return {
    fillStyle: '',
    font: '',
    textAlign: 'start',
    textBaseline: 'alphabetic',
    fillRect: vi.fn(),
    fillText: vi.fn(),
  } as unknown as CanvasRenderingContext2D;
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
  vi.restoreAllMocks();
  setLang('th');
});

function mount(store: DeskStore): void {
  dispose = render(() => step.component({ store }), host);
}

const q = <T extends Element>(sel: string): T | null => host.querySelector<T>(sel);
const button = (text: string): HTMLButtonElement | undefined =>
  [...host.querySelectorAll<HTMLButtonElement>('button')].find((b) => (b.textContent ?? '').includes(text));

function attach(file: File): void {
  const input = q<HTMLInputElement>('input[type="file"][name="passport"]');
  expect(input).not.toBeNull();
  Object.defineProperty(input!, 'files', { value: [file], configurable: true });
  input!.dispatchEvent(new Event('change', { bubbles: true }));
}

function typeField(name: string, value: string): void {
  const el = q<HTMLInputElement>(`input[name="${name}"]`);
  expect(el).not.toBeNull();
  el!.value = value;
  el!.dispatchEvent(new Event('input', { bubbles: true }));
  el!.dispatchEvent(new Event('change', { bubbles: true }));
}

function pickNationality(code: string): void {
  const sel = q<HTMLSelectElement>('select[name="nationality"]');
  expect(sel).not.toBeNull();
  sel!.value = code;
  sel!.dispatchEvent(new Event('change', { bubbles: true }));
}

function fillAll(): void {
  attach(JPEG);
  typeField('name', 'JOHN DOE');
  typeField('passportNo', 'AA1234567');
  pickNationality('USA');
  typeField('birthDate', '1990-01-01');
  typeField('passportExpiry', '2030-01-01');
}

describe('steps/customer: skip in-flight guard (#2315)', () => {
  it('two Skip clicks before the image is made: one reserveCustomer and one draftBegin', async () => {
    const pendingBlobs: BlobCallback[] = [];
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(stubCanvasContext() as never);
    vi.spyOn(HTMLCanvasElement.prototype, 'toBlob').mockImplementation((cb: BlobCallback) => {
      pendingBlobs.push(cb);
    });

    const s = rpcServer((a) => OK(a));
    const store = stubStore(STAFF_DESK, s.api);
    mount(store);

    const skip = button(SKIP)!;
    expect(skip).toBeDefined();
    expect(skip.disabled).toBe(false);

    skip.click();
    const skipDisabledWhileMakingImage = skip.disabled;

    // Second Skip click while the first image is still being made
    skip.click();

    for (const cb of pendingBlobs) {
      cb(new Blob([new Uint8Array([137, 80, 78, 71])], { type: 'image/png' }));
    }

    await vi.waitFor(() => expect(store.go).toHaveBeenCalledWith(2));
    await Promise.resolve();
    await Promise.resolve();

    expect(s.calls.filter((c) => c.action === 'draftBegin')).toHaveLength(1);
    expect(s.calls.filter((c) => c.action === 'reserveCustomer')).toHaveLength(1);
    expect(pendingBlobs).toHaveLength(1);
    expect(skipDisabledWhileMakingImage).toBe(true);
  });

  it('Skip, then Submit before the image is made: one reserve', async () => {
    const pendingBlobs: BlobCallback[] = [];
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(stubCanvasContext() as never);
    vi.spyOn(HTMLCanvasElement.prototype, 'toBlob').mockImplementation((cb: BlobCallback) => {
      pendingBlobs.push(cb);
    });

    const s = rpcServer((a) => OK(a));
    const store = stubStore(STAFF_DESK, s.api);
    mount(store);
    fillAll();

    const skip = button(SKIP)!;
    const submitBtn = q<HTMLButtonElement>('button[type="submit"]')!;
    expect(skip).toBeDefined();
    expect(submitBtn).not.toBeNull();

    skip.click();
    const skipDisabledWhileMakingImage = skip.disabled;
    const submitDisabledWhileMakingImage = submitBtn.disabled;

    // Clerk clicks Submit before toBlob finishes
    submitBtn.click();

    for (const cb of pendingBlobs) {
      cb(new Blob([new Uint8Array([137, 80, 78, 71])], { type: 'image/png' }));
    }

    await vi.waitFor(() => {
      expect(store.setPassport).toHaveBeenLastCalledWith(expect.objectContaining({ name: 'no-passport.png' }));
      expect(store.go).toHaveBeenCalledWith(2);
    });
    await Promise.resolve();
    await Promise.resolve();

    const reserves = s.calls.filter((c) => c.action === 'reserveCustomer');
    expect(reserves).toHaveLength(1);
    expect(s.calls.filter((c) => c.action === 'draftBegin')).toHaveLength(1);
    expect(reserves[0].payload.name).toBe('WALK-IN');
    expect(reserves[0].payload.passportNo).toBe('WALKIN');
    expect(reserves[0].payload.nationality).toBe('XXX');
    expect(skipDisabledWhileMakingImage).toBe(true);
    expect(submitDisabledWhileMakingImage).toBe(true);
  });

  it('image fails: skipFailed is shown and Skip is usable again', async () => {
    let callCount = 0;
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(stubCanvasContext() as never);
    vi.spyOn(HTMLCanvasElement.prototype, 'toBlob').mockImplementation((cb: BlobCallback) => {
      callCount += 1;
      if (callCount === 1) {
        throw new Error('canvas blocked');
      }
      cb(new Blob([new Uint8Array([137, 80, 78, 71])], { type: 'image/png' }));
    });

    const s = rpcServer((a) => OK(a));
    const store = stubStore(STAFF_DESK, s.api);
    mount(store);

    const skip = button(SKIP)!;
    expect(skip).toBeDefined();

    skip.click();
    const skipDisabledWhileMakingImage = skip.disabled;

    await vi.waitFor(() => {
      expect(q('[role="alert"]')?.textContent).toContain('ข้ามไม่สำเร็จ:');
    });
    expect(skip.disabled).toBe(false);
    expect(s.calls).toHaveLength(0);
    expect(store.go).not.toHaveBeenCalled();

    // Clicking Skip again succeeds and advances to step 2
    skip.click();
    await vi.waitFor(() => expect(store.go).toHaveBeenCalledWith(2));
    expect(q('[role="alert"]')).toBeNull();
    expect(s.calls.filter((c) => c.action === 'reserveCustomer')).toHaveLength(1);
    expect(skipDisabledWhileMakingImage).toBe(true);
  });
});
