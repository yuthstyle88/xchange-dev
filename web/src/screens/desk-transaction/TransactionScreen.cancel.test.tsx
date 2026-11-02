// B04 cancel #5933 (G5): the transaction screen cancels a CONFIRMED exchange with no document number
// (design docs/design/blueprint/desk-transaction-detail.md D12, PR #5931; owner 2026-11-02
// "ตามเสนอ เก็บกวาด d1d1a02c ผ่านแอปหลัง deploy"). Written before the implementation by claude/01TxaRJG (FE
// sub-controller). Do not edit, delete or weaken; add tests in a file of your own. The fake server, fixtures and helpers
// are TransactionScreen.test.tsx's, with the POST route generalised to the cancel path. One test per row; id in the name.
//
// The handle on the screen ([required] by these tests): the cancel button carries `data-action="cancel-transaction"`;
// it opens S-3's `ReasonDialog` (its <input>, its first button = back out, its last button = confirm).
// Amended 2026-11-02 by the author (claude/01TxaRJG), on gemini/studio-9's blocked report: C6 asserted the same key
// after a 409 refusal, but S-3's command.ts → keyedWrite drops the key on a 4xx refusal by design. C6 now asserts a new
// key after a refusal, and C6b the same key after a network failure.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { errorText, setLang } from '../../shared/i18n';
import {
  makeMe,
  makeTransactionAuditEventDto,
  makeTransactionAuditViewData,
  makeTransactionDetailDto,
  makeTransactionViewData,
  makeTxLineDto,
  makeTxPaymentDto,
  type MeView,
  type TransactionAuditViewData,
  type TransactionDetailDto,
  type TransactionViewData,
} from '../../shared/testing/fixtures';
import { th as shared } from '../../shared/i18n/th';
import { en, th } from './i18n';
import { TransactionScreen } from './TransactionScreen';

type TransactionData = TransactionViewData;
type TransactionAuditData = TransactionAuditViewData;

const TX_ID = '7f0c2b1e-4a5d-4c3b-9e8f-0a1b2c3d4e5f';
const PATH = `/desk/transactions/${TX_ID}`;
const CANCEL_PATH = `/api/transactions/${TX_ID}/cancel`;

// D12's words, verbatim.
const WORDS = {
  th: { cancelTransaction: 'ยกเลิกรายการ', cancelReasonTitle: 'เหตุผลที่ยกเลิกรายการ', cancelDone: 'ยกเลิกรายการแล้ว' },
  en: { cancelTransaction: 'Cancel transaction', cancelReasonTitle: 'Reason for cancelling', cancelDone: 'Transaction cancelled' },
} as const;

function me(permissions: string[]) {
  return makeMe({
    id: '00000000-0000-4000-8000-000000000001',
    name: 'คุณทดสอบ',
    branch: 'MAIN',
    branchName: 'สาขาทดสอบ',
    branches: [],
    permissions,
  });
}
const TELLER = ['TransactionRead', 'ExchangeTransact'];

/** A CONFIRMED exchange with no document number, as d1d1a02c on xc-v3. */
function confirmed(over: Partial<TransactionDetailDto> = {}) {
  return makeTransactionDetailDto({
    id: TX_ID,
    status: 'CONFIRMED',
    documentRef: null,
    branchId: 'MAIN',
    customerId: 'cust-1',
    quoteId: null,
    quoteExpiresAt: null,
    lines: [
      makeTxLineDto({
        lineNo: 1,
        currency: 'USD',
        side: 'BUY',
        quantity: '200.0000',
        unitSize: '1.0000',
        rateSnapshot: '33.65875000',
        thbAmount: '6731.75',
        feeThb: '0.00',
        settleThb: '6731.75',
      }),
    ],
    payment: makeTxPaymentDto({
      method: 'CASH',
      reference: '',
      account: '',
      slipFileKey: null,
      amountThb: '6731.75',
      bankFeeThb: '0.00',
      paidAt: '2026-11-02T05:51:35Z',
    }),
    receipts: [],
    approvalRequest: null,
    completedAt: null,
    cancelledAt: null,
    cancelReason: null,
    reversal: null,
    benefits: [],
    booking: null,
    ...over,
  });
}

const AUDIT: TransactionAuditData = makeTransactionAuditViewData({
  events: [makeTransactionAuditEventDto({ at: '2026-11-02T05:51:35Z', actorId: 'u-t', actorRole: 'TELLER', action: 'transaction.confirmed' })],
});

type Call = { method: string; path: string; body: string | null; key: string | null };
type Reply = { status: number; json?: unknown; network?: true };

interface World {
  me: MeView | Reply;
  transaction: TransactionData;
  /** Answers to the cancel POST, in order; the last one repeats. */
  posts: Reply[];
  /** After a successful POST, the transaction answer becomes this. */
  afterPost?: TransactionData;
}

let world: World;
let calls: Call[] = [];
let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let postCount = 0;

function isReply(v: unknown): v is Reply {
  return typeof v === 'object' && v !== null && 'status' in v && typeof (v as Reply).status === 'number';
}
function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
}
function view<T>(data: T | Reply): Response {
  if (isReply(data)) return json(data.status, data.json ?? {});
  return json(200, { version: 1, changed: true, data });
}

function installFetch() {
  calls = [];
  postCount = 0;
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(url, 'http://xc.test');
      const method = (init?.method ?? 'GET').toUpperCase();
      const headers = new Headers(init?.headers);
      const body = typeof init?.body === 'string' ? init.body : null;
      calls.push({ method, path: u.pathname, body, key: headers.get('Idempotency-Key') });
      if (method === 'GET' && u.pathname === '/api/v3/views/me') return view(world.me);
      if (method === 'GET' && u.pathname === '/api/v3/views/transaction') return view(world.transaction);
      if (method === 'GET' && u.pathname === '/api/v3/views/transaction_audit') return view(AUDIT);
      if (method === 'POST' && u.pathname === CANCEL_PATH) {
        const r = world.posts[Math.min(postCount, world.posts.length - 1)]!;
        postCount++;
        if (r.network) throw new TypeError('Failed to fetch');
        if (r.status < 300 && world.afterPost) world.transaction = world.afterPost;
        return json(r.status, r.json ?? {});
      }
      return json(404, { error: { code: 'notFound', message: `unexpected ${method} ${u.pathname}` } });
    }),
  );
}

async function settle(): Promise<void> {
  for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));
}

async function mount(over: Partial<World> = {}) {
  world = {
    me: me(TELLER),
    transaction: makeTransactionViewData({ transaction: confirmed() }),
    posts: [{ status: 200, json: { id: TX_ID, status: 'CANCELLED' } }],
    ...over,
  };
  installFetch();
  dispose = render(() => <TransactionScreen pathname={PATH} search="" />, host);
  await settle();
  return host;
}

const gets = (name: string) => calls.filter((c) => c.method === 'GET' && c.path === `/api/v3/views/${name}`);
const posts = () => calls.filter((c) => c.method === 'POST');
const cancelBtn = () => host.querySelectorAll<HTMLButtonElement>('button[data-action="cancel-transaction"]');
const text = () => (host.textContent ?? '').replace(/\s+/g, ' ');

async function click(el: Element | null | undefined) {
  expect(el, 'the button is there').toBeTruthy();
  (el as HTMLElement).click();
  await settle();
}

/** Opens the dialog through the cancel button, types `reason`, presses the dialog's last (confirm) or first button. */
async function cancelWith(reason: string, press: 'confirm' | 'back' = 'confirm') {
  await click(cancelBtn()[0]);
  const input = host.querySelector<HTMLInputElement>('dialog input');
  expect(input, 'S-3 ReasonDialog is open').not.toBeNull();
  input!.value = reason;
  input!.dispatchEvent(new Event('input', { bubbles: true }));
  const buttons = input!.closest('dialog')!.querySelectorAll('button');
  await click(press === 'confirm' ? buttons[buttons.length - 1] : buttons[0]);
}

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  setLang('th');
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  setLang('th');
});

describe('D12: cancel a CONFIRMED exchange with no document number', () => {
  it('C0: the three keys exist in th and en with the design wording', () => {
    for (const lang of ['th', 'en'] as const) {
      const table = (lang === 'th' ? th : en) as Record<string, string>;
      for (const [k, v] of Object.entries(WORDS[lang])) expect(table[k], `desk-transaction ${lang}.${k}`).toBe(v);
    }
  });

  it('C1: CONFIRMED, no documentRef, ExchangeTransact → one enabled "ยกเลิกรายการ" button', async () => {
    await mount();
    expect(cancelBtn()).toHaveLength(1);
    expect(cancelBtn()[0]!.disabled).toBe(false);
    expect(cancelBtn()[0]!.textContent?.trim()).toBe(WORDS.th.cancelTransaction);
  });

  const hidden: [string, Partial<World>][] = [
    ['COMPLETED', { transaction: makeTransactionViewData({ transaction: confirmed({ status: 'COMPLETED', documentRef: 'XC-0009' }) }) }],
    ['CONFIRMED with a documentRef', { transaction: makeTransactionViewData({ transaction: confirmed({ documentRef: 'XC-0009' }) }) }],
    ['PENDING_APPROVAL', { transaction: makeTransactionViewData({ transaction: confirmed({ status: 'PENDING_APPROVAL' }) }) }],
    ['CANCELLED', { transaction: makeTransactionViewData({ transaction: confirmed({ status: 'CANCELLED', cancelReason: 'x', cancelledAt: '2026-11-02T06:00:00Z' }) }) }],
    ['no ExchangeTransact', { me: me(['TransactionRead']) }],
  ];
  for (const [what, over] of hidden) {
    it(`C2: no cancel button when ${what}`, async () => {
      await mount(over);
      expect(cancelBtn()).toHaveLength(0);
    });
  }

  it('C3: backing out of the dialog sends nothing', async () => {
    await mount();
    await cancelWith('ลูกค้าไม่รอ', 'back');
    expect(posts()).toHaveLength(0);
  });

  it('C4: confirming sends one POST …/cancel {id, reason} with an Idempotency-Key', async () => {
    await mount();
    await cancelWith('ผู้จัดการไม่อนุมัติ ยอดติดลบ');
    expect(posts()).toHaveLength(1);
    const p = posts()[0]!;
    expect(p.path).toBe(CANCEL_PATH);
    expect(p.key, 'an Idempotency-Key').toBeTruthy();
    expect(JSON.parse(p.body ?? '{}')).toEqual({ id: TX_ID, reason: 'ผู้จัดการไม่อนุมัติ ยอดติดลบ' });
  });

  it('C5: success → "ยกเลิกรายการแล้ว", one re-read, the header shows CANCELLED with the reason, no button', async () => {
    await mount({
      afterPost: makeTransactionViewData({
        transaction: confirmed({ status: 'CANCELLED', cancelledAt: '2026-11-02T06:30:00Z', cancelReason: 'ผู้จัดการไม่อนุมัติ ยอดติดลบ' }),
      }),
    });
    const before = gets('transaction').length;
    await cancelWith('ผู้จัดการไม่อนุมัติ ยอดติดลบ');
    expect(text()).toContain(WORDS.th.cancelDone);
    expect(gets('transaction')).toHaveLength(before + 1);
    expect(text()).toContain('ผู้จัดการไม่อนุมัติ ยอดติดลบ');
    expect(cancelBtn()).toHaveLength(0);
  });

  it('C6: a refusal is shown as sent (worded); a new confirm after a 4xx refusal is a new request with a new key', async () => {
    await mount({
      posts: [
        { status: 409, json: { error: { code: 'staleVersion', message: 'staleVersion: a COMPLETED exchange is not cancelled' } } },
        { status: 200, json: { id: TX_ID, status: 'CANCELLED' } },
      ],
    });
    await cancelWith('x');
    expect(text()).toContain(errorText('staleVersion', 'staleVersion: a COMPLETED exchange is not cancelled'));
    await cancelWith('x');
    expect(posts()).toHaveLength(2);
    // keyedWrite (shared/api/keyed-write.ts:25-33, keyed-write.test.ts:92-107) drops the key after a 4xx refusal.
    expect(posts()[1]!.key).not.toBe(posts()[0]!.key);
  });

  it('C6b: after a network failure, the retry resends with the same key', async () => {
    await mount({ posts: [{ status: 0, network: true }, { status: 200, json: { id: TX_ID, status: 'CANCELLED' } }] });
    await cancelWith('x');
    await cancelWith('x');
    expect(posts()).toHaveLength(2);
    expect(posts()[1]!.key).toBe(posts()[0]!.key);
  });

  it('C7: me fails → the cancel button is shown disabled, with the reason as its title', async () => {
    const down: Reply = { status: 500, json: { error: { code: 'serverError', message: 'x' } } };
    await mount({ me: down });
    expect(cancelBtn()).toHaveLength(1);
    expect(cancelBtn()[0]!.disabled).toBe(true);
    expect(cancelBtn()[0]!.title).toContain(shared['shell.permissions.unavailable']);
  });
});
