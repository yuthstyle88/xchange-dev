import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiError, type ApiClient, type ViewAnswer, type ViewParams } from '../../../shared/api/client';
import type { TransactionRow } from '../../../shared/api/generated/transaction_row';
import { setLang } from '../../../shared/i18n';
import { en as sharedEn } from '../../../shared/i18n/en';
import { th as sharedTh } from '../../../shared/i18n/th';
import { Dialog, closeDialog, openDialog } from './Dialog';
import { NewTransactionButton } from './NewTransactionButton';
import { pendingWrite } from './pending-write';
import { Toast } from './toast';
import { TransactionDetail } from './TransactionDetail';

async function settle(): Promise<void> {
  for (let i = 0; i < 5; i++) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}

let dispose: (() => void) | undefined;
let root: HTMLElement;

function mount(ui: () => JSX.Element): HTMLElement {
  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(ui, root);
  return root;
}

interface Call {
  via: 'post' | 'view';
  path: string;
  body?: { action: string; payload: Record<string, unknown> };
  q?: ViewParams<{ requestId: string }>;
  resolve: (v: unknown) => void;
  reject: (e: unknown) => void;
}

class FakeApi {
  calls: Call[] = [];
  autoAnswer: unknown = undefined;

  readonly client = {
    post: (path: string, body: { action: string; payload: Record<string, unknown> }) => {
      if (this.autoAnswer !== undefined) return Promise.resolve(this.autoAnswer);
      return new Promise((resolve, reject) => {
        this.calls.push({ via: 'post', path, body: structuredClone(body), resolve, reject });
      });
    },
    view: (name: string, q: ViewParams<{ requestId: string }> = {}) =>
      new Promise((resolve, reject) => {
        this.calls.push({ via: 'view', path: name, q: structuredClone(q), resolve, reject });
      }),
    page: () => Promise.reject(new Error('reads no page')),
  };

  get api(): ApiClient {
    return this.client as unknown as ApiClient;
  }

  last(): Call {
    const c = this.calls[this.calls.length - 1];
    if (!c) throw new Error('no call yet');
    return c;
  }
}

let fake: FakeApi;

function openForm(): void {
  openDialog('ฟอร์มทดสอบ', () => (
    <form>
      <input name="test" value="123" />
      <button type="button">บันทึก</button>
    </form>
  ));
}

function toastText(): string | null {
  const status = document.querySelector('[role="status"]') as HTMLElement | null;
  if (!status || status.hidden || status.closest('[hidden]')) return null;
  const t = (status.textContent ?? '').trim();
  return t === '' ? null : t;
}

beforeEach(() => {
  fake = new FakeApi();
  setLang('th');
});

afterEach(async () => {
  for (const c of fake.calls) {
    try {
      c.resolve({ requestResolution: 'CANCELLED' });
    } catch {}
  }
  await settle();
  fake.autoAnswer = { requestResolution: 'CANCELLED' };
  try {
    await pendingWrite.abandon();
  } catch {}
  await settle();
  closeDialog();
  dispose?.();
  dispose = undefined;
  root?.remove();
  setLang('th');
  await settle();
});

function fakeTransactionRow(over: Partial<TransactionRow> = {}): TransactionRow {
  return {
    id: 'XC-B-2026-999999',
    date: '2026-10-15',
    created: '2026-10-15T05:00:00.000Z',
    type: 'BUY',
    currency: 'USD',
    amount: '100.00',
    rate: '35.0000',
    unitSize: '1',
    principal: '3500.00',
    fee: '0.00',
    bankFee: '0.00',
    settlement: '3500.00',
    method: 'Cash',
    customer: 'Test Customer',
    customerId: 'CUS-1',
    staff: 'teller1',
    status: 'COMPLETED',
    notes: null,
    cancelReason: null,
    shiftId: 'SH-1',
    referenceRate: null,
    referenceValue: null,
    referenceSpread: null,
    referencePercent: null,
    paymentRef: null,
    slipUrl: null,
    ...over,
  };
}

describe('WE-18: screens/admin/parts/ failed-call handling through wordedError', () => {
  // ── Catch 1: NewTransactionButton.tsx:55 (handleClick view admin_shift_state) ──
  describe('Catch 1: NewTransactionButton.tsx handleClick', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      const api = {
        view: vi.fn().mockRejectedValue(new TypeError('Failed to fetch')),
        post: vi.fn(),
        page: vi.fn(),
      } as unknown as ApiClient;

      const el = mount(() => (
        <>
          <NewTransactionButton api={api} branch="B1" />
          <Dialog />
          <Toast />
        </>
      ));

      const btn = el.querySelector<HTMLButtonElement>('.new-transaction-button')!;
      btn.click();
      await settle();

      const errorEl = el.querySelector('.read-error');
      expect(errorEl?.textContent).toBe(sharedTh['error.network']);

      // EN
      setLang('en');
      dispose?.();
      dispose = undefined;
      root.remove();

      const elEn = mount(() => (
        <>
          <NewTransactionButton api={api} branch="B1" />
          <Dialog />
          <Toast />
        </>
      ));
      const btnEn = elEn.querySelector<HTMLButtonElement>('.new-transaction-button')!;
      btnEn.click();
      await settle();
      expect(elEn.querySelector('.read-error')?.textContent).toBe(sharedEn['error.network']);
    });

    it('(2) 500 refusal with keyed non-server code shows error.serverError', async () => {
      const api = {
        view: vi.fn().mockRejectedValue(
          new ApiError({
            status: 500,
            code: 'roleNotAllowedHere',
            message: 'Internal server failure',
          }),
        ),
        post: vi.fn(),
        page: vi.fn(),
      } as unknown as ApiClient;

      const el = mount(() => (
        <>
          <NewTransactionButton api={api} branch="B1" />
          <Dialog />
          <Toast />
        </>
      ));

      const btn = el.querySelector<HTMLButtonElement>('.new-transaction-button')!;
      btn.click();
      await settle();

      const errorEl = el.querySelector('.read-error');
      expect(errorEl?.textContent).toBe(sharedTh['error.serverError']);
    });

    it('(3) refusal with unmapped code and server message shows that message', async () => {
      const serverMessage = 'กะนี้ยังไม่สามารถทำรายการได้เนื่องจากอยู่นอกเวลา';
      const api = {
        view: vi.fn().mockRejectedValue(
          new ApiError({
            status: 400,
            code: 'unmappedShiftError',
            message: serverMessage,
          }),
        ),
        post: vi.fn(),
        page: vi.fn(),
      } as unknown as ApiClient;

      const el = mount(() => (
        <>
          <NewTransactionButton api={api} branch="B1" />
          <Dialog />
          <Toast />
        </>
      ));

      const btn = el.querySelector<HTMLButtonElement>('.new-transaction-button')!;
      btn.click();
      await settle();

      const errorEl = el.querySelector('.read-error');
      expect(errorEl?.textContent).toBe(serverMessage);
    });
  });

  // ── Catch 2: TransactionDetail.tsx:66 (onMount view transaction_documents) ──
  describe('Catch 2: TransactionDetail.tsx onMount documents read', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      const api = {
        view: vi.fn().mockRejectedValue(new TypeError('Failed to fetch')),
        post: vi.fn(),
        page: vi.fn(),
      } as unknown as ApiClient;

      const el = mount(() => (
        <TransactionDetail
          api={api}
          branch="B1"
          row={fakeTransactionRow()}
          onClose={vi.fn()}
        />
      ));
      await settle();

      const alertEl = el.querySelector('[role="alert"]') ?? el.querySelector('.receipt-section');
      expect(alertEl?.textContent).toContain(sharedTh['error.network']);

      // EN
      setLang('en');
      dispose?.();
      dispose = undefined;
      root.remove();

      const elEn = mount(() => (
        <TransactionDetail
          api={api}
          branch="B1"
          row={fakeTransactionRow()}
          onClose={vi.fn()}
        />
      ));
      await settle();

      const alertElEn = elEn.querySelector('[role="alert"]') ?? elEn.querySelector('.receipt-section');
      expect(alertElEn?.textContent).toContain(sharedEn['error.network']);
    });

    it('(2) 500 refusal with keyed non-server code shows error.serverError', async () => {
      const api = {
        view: vi.fn().mockRejectedValue(
          new ApiError({
            status: 500,
            code: 'roleNotAllowedHere',
            message: 'Database crash',
          }),
        ),
        post: vi.fn(),
        page: vi.fn(),
      } as unknown as ApiClient;

      const el = mount(() => (
        <TransactionDetail
          api={api}
          branch="B1"
          row={fakeTransactionRow()}
          onClose={vi.fn()}
        />
      ));
      await settle();

      const alertEl = el.querySelector('[role="alert"]') ?? el.querySelector('.receipt-section');
      expect(alertEl?.textContent).toContain(sharedTh['error.serverError']);
    });

    it('(3) refusal with unmapped code and server message shows that message', async () => {
      const serverMessage = 'เอกสารใบเสร็จถูกระงับการเข้าถึง';
      const api = {
        view: vi.fn().mockRejectedValue(
          new ApiError({
            status: 400,
            code: 'unmappedDocumentError',
            message: serverMessage,
          }),
        ),
        post: vi.fn(),
        page: vi.fn(),
      } as unknown as ApiClient;

      const el = mount(() => (
        <TransactionDetail
          api={api}
          branch="B1"
          row={fakeTransactionRow()}
          onClose={vi.fn()}
        />
      ));
      await settle();

      const alertEl = el.querySelector('[role="alert"]') ?? el.querySelector('.receipt-section');
      expect(alertEl?.textContent).toContain(serverMessage);
    });
  });

  // ── Catch 3: pending-write.tsx:158 (pendingWrite.run post /api/rpc) ──
  describe('Catch 3: pendingWrite.run failure', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      mount(() => (
        <>
          <Dialog />
          <Toast />
        </>
      ));
      openForm();

      void pendingWrite.run(fake.api, 'B1', 'adminTest', {}, vi.fn());
      await settle();
      fake.last().reject(new TypeError('Failed to fetch'));
      await settle();

      const dialogEl = document.querySelector('dialog');
      expect(dialogEl?.querySelector('.error')?.textContent).toBe(sharedTh['error.network']);

      // EN
      setLang('en');
      fake.autoAnswer = { requestResolution: 'CANCELLED' };
      await pendingWrite.abandon();
      await settle();
      fake.autoAnswer = undefined;
      closeDialog();
      openForm();
      void pendingWrite.run(fake.api, 'B1', 'adminTest2', {}, vi.fn());
      await settle();
      fake.last().reject(new TypeError('Failed to fetch'));
      await settle();
      expect(document.querySelector('dialog')?.querySelector('.error')?.textContent).toBe(sharedEn['error.network']);
    });

    it('(2) 500 refusal with keyed non-server code shows error.serverError', async () => {
      mount(() => (
        <>
          <Dialog />
          <Toast />
        </>
      ));
      openForm();

      void pendingWrite.run(fake.api, 'B1', 'adminTest', {}, vi.fn());
      await settle();
      fake.last().reject(
        new ApiError({
          status: 500,
          code: 'roleNotAllowedHere',
          message: 'Server error on write',
        }),
      );
      await settle();

      const dialogEl = document.querySelector('dialog');
      expect(dialogEl?.querySelector('.error')?.textContent).toBe(sharedTh['error.serverError']);
    });

    it('(3) refusal with unmapped code and server message shows that message', async () => {
      const serverMessage = 'ยอดเงินสดไม่พอสำหรับการบันทึกรายการ';
      mount(() => (
        <>
          <Dialog />
          <Toast />
        </>
      ));
      openForm();

      void pendingWrite.run(fake.api, 'B1', 'adminTest', {}, vi.fn());
      await settle();
      fake.last().reject(
        new ApiError({
          status: 400,
          code: 'unmappedWriteRefusal',
          message: serverMessage,
        }),
      );
      await settle();

      const dialogEl = document.querySelector('dialog');
      expect(dialogEl?.querySelector('.error')?.textContent).toBe(serverMessage);
    });
  });

  // ── Catch 4: pending-write.tsx:192 (pendingWrite.retry post /api/rpc) ──
  describe('Catch 4: pendingWrite.retry failure', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      mount(() => (
        <>
          <Dialog />
          <Toast />
        </>
      ));
      openForm();

      void pendingWrite.run(fake.api, 'B1', 'adminRetryAction', {}, vi.fn());
      await settle();
      fake.last().reject(new ApiError({ status: 503, code: 'init', message: 'First fail' }));
      await settle();

      // Click retry
      const retryBtn = [...document.querySelectorAll<HTMLButtonElement>('dialog button')].find(
        (b) => b.textContent?.includes('ลองคำขอเดิมซ้ำ'),
      )!;
      retryBtn.click();
      await settle();
      fake.last().reject(new TypeError('Network offline'));
      await settle();

      const dialogEl = document.querySelector('dialog');
      expect(dialogEl?.querySelector('.error')?.textContent).toBe(sharedTh['error.network']);

      // EN
      setLang('en');
      const retryBtnEn = [...document.querySelectorAll<HTMLButtonElement>('dialog button')].find(
        (b) => b.textContent?.includes('ลองคำขอเดิมซ้ำ') || b.textContent?.includes('Retry'),
      )!;
      retryBtnEn.click();
      await settle();
      fake.last().reject(new TypeError('Network offline'));
      await settle();
      expect(document.querySelector('dialog')?.querySelector('.error')?.textContent).toBe(sharedEn['error.network']);
    });

    it('(2) 500 refusal with keyed non-server code shows error.serverError', async () => {
      mount(() => (
        <>
          <Dialog />
          <Toast />
        </>
      ));
      openForm();

      void pendingWrite.run(fake.api, 'B1', 'adminRetryAction', {}, vi.fn());
      await settle();
      fake.last().reject(new ApiError({ status: 503, code: 'init', message: 'First fail' }));
      await settle();

      const retryBtn = [...document.querySelectorAll<HTMLButtonElement>('dialog button')].find(
        (b) => b.textContent?.includes('ลองคำขอเดิมซ้ำ'),
      )!;
      retryBtn.click();
      await settle();
      fake.last().reject(
        new ApiError({
          status: 500,
          code: 'roleNotAllowedHere',
          message: 'Retry 500 fail',
        }),
      );
      await settle();

      const dialogEl = document.querySelector('dialog');
      expect(dialogEl?.querySelector('.error')?.textContent).toBe(sharedTh['error.serverError']);
    });

    it('(3) refusal with unmapped code and server message shows that message', async () => {
      const serverMessage = 'ไม่สามารถลองคำขอนี้ซ้ำได้ เนื่องจากระบบกำลังประมวลผล';
      mount(() => (
        <>
          <Dialog />
          <Toast />
        </>
      ));
      openForm();

      void pendingWrite.run(fake.api, 'B1', 'adminRetryAction', {}, vi.fn());
      await settle();
      fake.last().reject(new ApiError({ status: 503, code: 'init', message: 'First fail' }));
      await settle();

      const retryBtn = [...document.querySelectorAll<HTMLButtonElement>('dialog button')].find(
        (b) => b.textContent?.includes('ลองคำขอเดิมซ้ำ'),
      )!;
      retryBtn.click();
      await settle();
      fake.last().reject(
        new ApiError({
          status: 400,
          code: 'unmappedRetryRefusal',
          message: serverMessage,
        }),
      );
      await settle();

      const dialogEl = document.querySelector('dialog');
      expect(dialogEl?.querySelector('.error')?.textContent).toBe(serverMessage);
    });
  });

  // ── Catch 5: pending-write.tsx:230 (pendingWrite.inspect view admin_request_status) ──
  describe('Catch 5: pendingWrite.inspect failure', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      mount(() => (
        <>
          <Dialog />
          <Toast />
        </>
      ));
      openForm();

      void pendingWrite.run(fake.api, 'B1', 'adminInspectAction', { requestId: 'REQ-1' }, vi.fn());
      await settle();
      fake.last().reject(new ApiError({ status: 503, code: 'init', message: 'Initial' }));
      await settle();

      const inspectBtn = [...document.querySelectorAll<HTMLButtonElement>('dialog button')].find(
        (b) => b.textContent?.includes('ตรวจสถานะล่าสุด'),
      )!;
      inspectBtn.click();
      await settle();
      fake.last().reject(new TypeError('Failed to fetch'));
      await settle();

      expect(toastText()).toBe(sharedTh['error.network']);

      // EN
      setLang('en');
      inspectBtn.click();
      await settle();
      fake.last().reject(new TypeError('Failed to fetch'));
      await settle();
      expect(toastText()).toBe(sharedEn['error.network']);
    });

    it('(2) 500 refusal with keyed non-server code shows error.serverError', async () => {
      mount(() => (
        <>
          <Dialog />
          <Toast />
        </>
      ));
      openForm();

      void pendingWrite.run(fake.api, 'B1', 'adminInspectAction', { requestId: 'REQ-1' }, vi.fn());
      await settle();
      fake.last().reject(new ApiError({ status: 503, code: 'init', message: 'Initial' }));
      await settle();

      const inspectBtn = [...document.querySelectorAll<HTMLButtonElement>('dialog button')].find(
        (b) => b.textContent?.includes('ตรวจสถานะล่าสุด'),
      )!;
      inspectBtn.click();
      await settle();
      fake.last().reject(
        new ApiError({
          status: 500,
          code: 'roleNotAllowedHere',
          message: 'Inspect server fail',
        }),
      );
      await settle();

      expect(toastText()).toBe(sharedTh['error.serverError']);
    });

    it('(3) refusal with unmapped code and server message shows that message', async () => {
      const serverMessage = 'ไม่สามารถตรวจสอบสถานะคำขอนี้ได้ในขณะนี้';
      mount(() => (
        <>
          <Dialog />
          <Toast />
        </>
      ));
      openForm();

      void pendingWrite.run(fake.api, 'B1', 'adminInspectAction', { requestId: 'REQ-1' }, vi.fn());
      await settle();
      fake.last().reject(new ApiError({ status: 503, code: 'init', message: 'Initial' }));
      await settle();

      const inspectBtn = [...document.querySelectorAll<HTMLButtonElement>('dialog button')].find(
        (b) => b.textContent?.includes('ตรวจสถานะล่าสุด'),
      )!;
      inspectBtn.click();
      await settle();
      fake.last().reject(
        new ApiError({
          status: 400,
          code: 'unmappedInspectRefusal',
          message: serverMessage,
        }),
      );
      await settle();

      expect(toastText()).toBe(serverMessage);
    });
  });

  // ── Catch 6: pending-write.tsx:267 (pendingWrite.abandon post /api/rpc) ──
  describe('Catch 6: pendingWrite.abandon failure', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      mount(() => (
        <>
          <Dialog />
          <Toast />
        </>
      ));
      openForm();

      void pendingWrite.run(fake.api, 'B1', 'adminAbandonAction', { requestId: 'REQ-1' }, vi.fn());
      await settle();
      fake.last().reject(new ApiError({ status: 503, code: 'init', message: 'Initial' }));
      await settle();

      // Click inspect to transition to notFound dialog
      const inspectBtn = [...document.querySelectorAll<HTMLButtonElement>('dialog button')].find(
        (b) => b.textContent?.includes('ตรวจสถานะล่าสุด'),
      )!;
      inspectBtn.click();
      await settle();
      fake.last().resolve({ version: 1, changed: true, data: { applied: false } } satisfies ViewAnswer<{ applied: boolean }>);
      await settle();

      // Click discard (abandon)
      const discardBtn = [...document.querySelectorAll<HTMLButtonElement>('dialog button')].find(
        (b) => b.textContent?.includes('ยกเลิกคำขอที่ยังไม่บันทึก'),
      )!;
      discardBtn.click();
      await settle();
      fake.last().reject(new TypeError('Failed to fetch'));
      await settle();

      expect(toastText()).toBe(sharedTh['error.network']);

      // EN
      setLang('en');
      discardBtn.click();
      await settle();
      fake.last().reject(new TypeError('Failed to fetch'));
      await settle();
      expect(toastText()).toBe(sharedEn['error.network']);
    });

    it('(2) 500 refusal with keyed non-server code shows error.serverError', async () => {
      mount(() => (
        <>
          <Dialog />
          <Toast />
        </>
      ));
      openForm();

      void pendingWrite.run(fake.api, 'B1', 'adminAbandonAction', { requestId: 'REQ-1' }, vi.fn());
      await settle();
      fake.last().reject(new ApiError({ status: 503, code: 'init', message: 'Initial' }));
      await settle();

      const inspectBtn = [...document.querySelectorAll<HTMLButtonElement>('dialog button')].find(
        (b) => b.textContent?.includes('ตรวจสถานะล่าสุด'),
      )!;
      inspectBtn.click();
      await settle();
      fake.last().resolve({ version: 1, changed: true, data: { applied: false } } satisfies ViewAnswer<{ applied: boolean }>);
      await settle();

      const discardBtn = [...document.querySelectorAll<HTMLButtonElement>('dialog button')].find(
        (b) => b.textContent?.includes('ยกเลิกคำขอที่ยังไม่บันทึก'),
      )!;
      discardBtn.click();
      await settle();
      fake.last().reject(
        new ApiError({
          status: 500,
          code: 'roleNotAllowedHere',
          message: 'Abandon 500 error',
        }),
      );
      await settle();

      expect(toastText()).toBe(sharedTh['error.serverError']);
    });

    it('(3) refusal with unmapped code and server message shows that message', async () => {
      const serverMessage = 'ไม่สามารถยกเลิกคำขอนี้ได้ในขณะนี้';
      mount(() => (
        <>
          <Dialog />
          <Toast />
        </>
      ));
      openForm();

      void pendingWrite.run(fake.api, 'B1', 'adminAbandonAction', { requestId: 'REQ-1' }, vi.fn());
      await settle();
      fake.last().reject(new ApiError({ status: 503, code: 'init', message: 'Initial' }));
      await settle();

      const inspectBtn = [...document.querySelectorAll<HTMLButtonElement>('dialog button')].find(
        (b) => b.textContent?.includes('ตรวจสถานะล่าสุด'),
      )!;
      inspectBtn.click();
      await settle();
      fake.last().resolve({ version: 1, changed: true, data: { applied: false } } satisfies ViewAnswer<{ applied: boolean }>);
      await settle();

      const discardBtn = [...document.querySelectorAll<HTMLButtonElement>('dialog button')].find(
        (b) => b.textContent?.includes('ยกเลิกคำขอที่ยังไม่บันทึก'),
      )!;
      discardBtn.click();
      await settle();
      fake.last().reject(
        new ApiError({
          status: 400,
          code: 'unmappedAbandonRefusal',
          message: serverMessage,
        }),
      );
      await settle();

      expect(toastText()).toBe(serverMessage);
    });
  });

  // ── Done criterion 3: Stale data cleared on branch switch ──
  describe('Done criterion 3: Stale data cleared on branch switch', () => {
    it('NewTransactionButton: read error is cleared when branch changes', async () => {
      const api = {
        view: vi.fn().mockRejectedValue(new TypeError('Failed to fetch')),
        post: vi.fn(),
        page: vi.fn(),
      } as unknown as ApiClient;

      const [branch, setBranch] = createSignal('B1');
      const el = mount(() => (
        <>
          <NewTransactionButton api={api} branch={branch()} />
          <Dialog />
          <Toast />
        </>
      ));

      const btn = el.querySelector<HTMLButtonElement>('.new-transaction-button')!;
      btn.click();
      await settle();

      expect(el.querySelector('.read-error')?.textContent).toBe(sharedTh['error.network']);

      // Branch switches to B2
      setBranch('B2');
      await settle();

      expect(el.querySelector('.read-error')).toBeNull();
    });
  });
});
