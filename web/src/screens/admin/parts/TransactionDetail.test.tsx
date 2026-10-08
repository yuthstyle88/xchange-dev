// V2-parts (#476): the LIVE exchange detail with its slip and receipt sections (ticket §7
// `TransactionDetail.tsx`, §9, design §12.2, §12.14, behaviour P2, P6). Written before the
// implementation by the lane controller (claude/7ca80c). Do not edit, delete or weaken; add tests
// in a file of your own.
//
// v2 (`xchange-v2-source@4d0bdbb`; decoded admin page `admin-html:N`, `src/online/public/slips.js:N`):
// - fields (`admin-html:77`), in this order, each a `<dt>` with its `<dd>`: เลขรายการ, สถานะ, ลูกค้า
//   (the LIVE label, `:133`), วันที่ / เวลา (`date` + Bangkok HH:MM of `created`), ประเภท, จำนวนเงิน
//   (amount + currency), เรท, เงินต้น, ค่าบริการ, ร้านจ่ายลูกค้า (BUY) / ร้านรับจากลูกค้า (SELL) — the
//   settlement, ค่าธรรมเนียมธนาคาร, ช่องทาง, พนักงาน, หมายเหตุ (`—` when none); then, only when the
//   row has a reference rate, เรทอ้างอิง, มูลค่าอ้างอิง, ส่วนต่างเทียบเรทอ้างอิง (`<spread> THB
//   (<percent>%)`); then เหตุผลยกเลิก when the row has one;
// - the LIVE note `ข้อมูลจาก เซิร์ฟเวอร์ · รายละเอียดนี้ไม่ใช่ใบเสร็จ` (`:133`); for a COMPLETED row the
//   cancel button shown **disabled** as `รายการจริง: ตรวจสอบการคืนเงินกับผู้ดูแล` (`:133`), none for
//   any other status; `ปิด` calls `onClose`;
// - the slip section `สลิป / หลักฐานรับ–จ่าย` (`slips.js:18`): `เปิดสลิป ↗` (new tab) and
//   `ดาวน์โหลด` (`<slipUrl>?download=1`), or `รายการนี้ไม่มีสลิปแนบ` (`slips.js:5`, `:18`);
// - the receipt section `ใบเสร็จ` (`slips.js:23-55`), read from the view `transaction_documents`
//   with the filter `{ref: <id>}` on the part's branch (never `/documents` HTML, done criterion
//   3): `กำลังโหลดใบเสร็จของ <id>…` until it answers, then `เอกสารที่บันทึกไว้ · <id>` and per file an
//   open link (`เปิดดู ใบเสร็จ` / `เปิดดู ภาพใบเสร็จ`, new tab) and, when the file has a download
//   name, a download link carrying it; the receipt in a frame with an empty `sandbox`, titled
//   `ใบเสร็จ <id>`; a path not under `/files/` is never linked nor framed (`slips.js:44`, `:53`).
//   Failures: a refused read → `โหลดใบเสร็จไม่ได้ กรุณาเข้าสู่ระบบแล้วลองใหม่` (`slips.js:35`); no file →
//   `ยังไม่มีใบเสร็จที่บันทึกไว้สำหรับรายการนี้` (`slips.js:38`); closed before the answer → nothing
//   is drawn into it (`slips.js:39`, §9).
//
// Figures (W7, P6): every figure is the row's string — the rows below are chosen so that nothing
// the screen could compute matches, and one figure is beyond what a JavaScript number holds. The
// percentage is the server's `referencePercent` as written.
//
// The title of the dialog this body sits in (`รายละเอียดรายการ`) is the opener's, not pinned here.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { JSX } from 'solid-js';
import { ApiError, type ApiClient, type ViewAnswer, type ViewParams } from '../../../shared/api/client';
import type { TransactionRow } from '../../../shared/api/generated/transaction_row';
import { setLang } from '../../../shared/i18n';
import { formatMoney } from '../../../shared/ui/MoneyText';
import { TransactionDetail } from './TransactionDetail';

const THAI = /[฀-๿]/;
const BIG = '90071992547409.93';

function row(over: Partial<TransactionRow> = {}): TransactionRow {
  return {
    id: 'XC-B-2026-000042',
    date: '2026-10-08',
    created: '2026-10-08T03:04:05.000Z',
    type: 'BUY',
    currency: 'USD',
    amount: '100.00',
    rate: '32.1000',
    principal: BIG,
    fee: '15.00',
    bankFee: '2.50',
    settlement: '3195.55',
    method: 'Cash',
    customer: 'Somchai Jaidee',
    customerId: 'CUS-7',
    staff: 'front1',
    status: 'COMPLETED',
    notes: null,
    cancelReason: null,
    shiftId: 'SH-1',
    referenceRate: '32.4000',
    referenceValue: '3240.00',
    referenceSpread: '30.00',
    referencePercent: '0.925926',
    paymentRef: null,
    slipUrl: null,
    ...over,
  };
}

// ── a fake ApiClient: `view` waits until the test answers ─────────────────────

interface Read {
  name: string;
  q: ViewParams<{ ref: string }>;
}

interface Pending {
  resolve: (answer: ViewAnswer<unknown>) => void;
  reject: (e: unknown) => void;
}

function fakeApi(): { api: ApiClient; reads: Read[]; pending: Pending[] } {
  const reads: Read[] = [];
  const pending: Pending[] = [];
  const api = {
    view: (name: string, q: ViewParams<{ ref: string }> = {}) => {
      reads.push({ name, q });
      return new Promise<ViewAnswer<unknown>>((resolve, reject) => {
        pending.push({ resolve, reject });
      });
    },
    page: async () => {
      throw new Error('the detail reads no page');
    },
    post: async () => {
      throw new Error('the detail writes nothing');
    },
  };
  return { api: api as unknown as ApiClient, reads, pending };
}

function documents(ref: string, files: Array<{ kind: string; path: string; download?: string }>): ViewAnswer<unknown> {
  return { version: 7, changed: true, data: { ref, files } };
}

async function settle(): Promise<void> {
  for (let i = 0; i < 5; i++) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}

let dispose: (() => void) | undefined;
let root: HTMLElement;
let fetchSpy: ReturnType<typeof vi.fn>;

function mount(fn: () => JSX.Element): HTMLElement {
  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(fn, root);
  return root;
}

beforeEach(() => {
  fetchSpy = vi.fn(() => Promise.reject(new Error('the detail never calls fetch')));
  vi.stubGlobal('fetch', fetchSpy);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  root?.remove();
  vi.unstubAllGlobals();
  setLang('th');
});

function fields(el: HTMLElement): Array<[string, string]> {
  const dts = [...el.querySelectorAll('dt')];
  return dts.map((dt) => {
    const dd = dt.nextElementSibling;
    expect(dd?.tagName, `${dt.textContent}: a dd follows`).toBe('DD');
    return [dt.textContent ?? '', (dd!.textContent ?? '').trim()];
  });
}

function buttonsNamed(el: HTMLElement, text: string): HTMLButtonElement[] {
  return [...el.querySelectorAll('button')].filter((b) => (b.textContent ?? '').trim() === text) as HTMLButtonElement[];
}

function linksNamed(el: HTMLElement, text: string): HTMLAnchorElement[] {
  return [...el.querySelectorAll('a')].filter((a) => (a.textContent ?? '').trim() === text);
}

function show(r: TransactionRow, onClose: () => void = () => {}) {
  const { api, reads, pending } = fakeApi();
  const el = mount(() => <TransactionDetail api={api} branch="B1" row={r} onClose={onClose} />);
  return { el, reads, pending };
}

describe('TransactionDetail — the fields (#476 P2, P6)', () => {
  it("v2's LIVE fields in order, with the server's figures", () => {
    const { el } = show(row());
    expect(fields(el)).toEqual([
      ['เลขรายการ', 'XC-B-2026-000042'],
      ['สถานะ', 'สำเร็จ'],
      ['ลูกค้า', 'Somchai Jaidee'],
      ['วันที่ / เวลา', '2026-10-08 10:04'],
      ['ประเภท', 'ร้านรับซื้อ'],
      ['จำนวนเงิน', '100.00 USD'],
      ['เรท', '32.1000'],
      ['เงินต้น', `${formatMoney(BIG)} THB`],
      ['ค่าบริการ', '15.00 THB'],
      ['ร้านจ่ายลูกค้า', '3,195.55 THB'],
      ['ค่าธรรมเนียมธนาคาร', '2.50 THB'],
      ['ช่องทาง', 'เงินสด'],
      ['พนักงาน', 'front1'],
      ['หมายเหตุ', '—'],
      ['เรทอ้างอิง', '32.4000'],
      ['มูลค่าอ้างอิง', '3,240.00 THB'],
      ['ส่วนต่างเทียบเรทอ้างอิง', '30.00 THB (0.925926%)'],
    ]);
    expect(el.textContent).toContain('90,071,992,547,409.93');
  });

  it('a SELL: ร้านรับจากลูกค้า holds the settlement; notes are shown as written', () => {
    const { el } = show(row({ type: 'SELL', settlement: '3224.45', notes: 'ลูกค้าประจำ' }));
    const f = Object.fromEntries(fields(el));
    expect(f['ร้านรับจากลูกค้า']).toBe('3,224.45 THB');
    expect(f['ร้านจ่ายลูกค้า']).toBeUndefined();
    expect(f['ประเภท']).toBe('ร้านขาย');
    expect(f['หมายเหตุ']).toBe('ลูกค้าประจำ');
  });

  it('no reference rate: no reference rows (admin-html:77)', () => {
    const { el } = show(row({ referenceRate: null, referenceValue: null, referenceSpread: null, referencePercent: null }));
    const labels = fields(el).map(([k]) => k);
    expect(labels).not.toContain('เรทอ้างอิง');
    expect(labels).not.toContain('มูลค่าอ้างอิง');
    expect(labels).not.toContain('ส่วนต่างเทียบเรทอ้างอิง');
    expect(labels[labels.length - 1]).toBe('หมายเหตุ');
  });

  it('a CANCELLED row: ยกเลิก, เหตุผลยกเลิก last, and no cancel button (§9)', () => {
    const { el } = show(row({ status: 'CANCELLED', cancelReason: 'ลูกค้าเปลี่ยนใจ' }));
    const f = fields(el);
    expect(f[1]).toEqual(['สถานะ', 'ยกเลิก']);
    expect(f[f.length - 1]).toEqual(['เหตุผลยกเลิก', 'ลูกค้าเปลี่ยนใจ']);
    expect(buttonsNamed(el, 'รายการจริง: ตรวจสอบการคืนเงินกับผู้ดูแล')).toEqual([]);
  });

  it('a COMPLETED row: the cancel button is there and disabled, with the LIVE text (admin-html:133)', () => {
    const { el } = show(row());
    const cancel = buttonsNamed(el, 'รายการจริง: ตรวจสอบการคืนเงินกับผู้ดูแล');
    expect(cancel.length).toBe(1);
    expect(cancel[0]!.disabled).toBe(true);
  });

  it('a PENDING row: รอบันทึกเอกสาร and no cancel button', () => {
    const { el } = show(row({ status: 'PENDING' }));
    expect(fields(el)[1]).toEqual(['สถานะ', 'รอบันทึกเอกสาร']);
    expect(buttonsNamed(el, 'รายการจริง: ตรวจสอบการคืนเงินกับผู้ดูแล')).toEqual([]);
  });

  it('the LIVE note, and none of the DEMO wording (§12.5)', () => {
    const { el } = show(row());
    const text = el.textContent ?? '';
    expect(text).toContain('ข้อมูลจาก เซิร์ฟเวอร์ · รายละเอียดนี้ไม่ใช่ใบเสร็จ');
    for (const demo of ['ข้อมูลทดลอง', 'ลูกค้าทดลอง', 'ยกเลิกรายการทดลอง']) {
      expect(text.includes(demo), demo).toBe(false);
    }
  });

  it('ปิด calls onClose', () => {
    const onClose = vi.fn();
    const { el } = show(row(), onClose);
    const close = buttonsNamed(el, 'ปิด');
    expect(close.length).toBe(1);
    close[0]!.click();
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});

describe('TransactionDetail — the slip section (#476 P2, slips.js:18)', () => {
  it('a slip: open in a new tab and download', () => {
    const { el } = show(row({ slipUrl: '/files/slip-abc' }));
    const headings = [...el.querySelectorAll('*')].filter((n) => (n.textContent ?? '').trim() === 'สลิป / หลักฐานรับ–จ่าย');
    expect(headings.length).toBe(1);
    const open = linksNamed(el, 'เปิดสลิป ↗');
    expect(open.length).toBe(1);
    expect(open[0]!.getAttribute('href')).toBe('/files/slip-abc');
    expect(open[0]!.target).toBe('_blank');
    expect(open[0]!.rel).toContain('noopener');
    const download = linksNamed(el, 'ดาวน์โหลด');
    expect(download.length).toBe(1);
    expect(download[0]!.getAttribute('href')).toBe('/files/slip-abc?download=1');
    expect(el.textContent).not.toContain('รายการนี้ไม่มีสลิปแนบ');
  });

  it('no slip: รายการนี้ไม่มีสลิปแนบ and no slip link (§9)', () => {
    const { el } = show(row({ slipUrl: null }));
    expect(el.textContent).toContain('สลิป / หลักฐานรับ–จ่าย');
    expect(el.textContent).toContain('รายการนี้ไม่มีสลิปแนบ');
    expect(linksNamed(el, 'เปิดสลิป ↗')).toEqual([]);
  });
});

describe('TransactionDetail — the receipt section (#476 P2, §12.14, slips.js:23-55)', () => {
  it('reads transaction_documents once, for this ref on this branch, through the ApiClient', () => {
    const { reads } = show(row({ id: 'TX-9' }));
    expect(reads.length).toBe(1);
    expect(reads[0]!.name).toBe('transaction_documents');
    expect(reads[0]!.q.branch).toBe('B1');
    expect(reads[0]!.q.filter).toEqual({ ref: 'TX-9' });
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it('the heading ใบเสร็จ and the loading line until the view answers', () => {
    const { el } = show(row({ id: 'TX-9' }));
    const headings = [...el.querySelectorAll('*')].filter((n) => (n.textContent ?? '').trim() === 'ใบเสร็จ');
    expect(headings.length).toBe(1);
    expect(el.textContent).toContain('กำลังโหลดใบเสร็จของ TX-9…');
    expect(el.querySelector('iframe')).toBeNull();
  });

  it('a receipt: the saved line, an open link, a download link and the sandboxed frame', async () => {
    const { el, pending } = show(row({ id: 'TX-9' }));
    pending[0]!.resolve(
      documents('TX-9', [{ kind: 'receipt', path: '/files/receipt-abc', download: 'TX-9_RECEIPT_FULL.html' }]),
    );
    await settle();
    const text = el.textContent ?? '';
    expect(text).toContain('เอกสารที่บันทึกไว้ · TX-9');
    expect(text).not.toContain('กำลังโหลดใบเสร็จของ TX-9…');
    const open = linksNamed(el, 'เปิดดู ใบเสร็จ');
    expect(open.length).toBe(1);
    expect(open[0]!.getAttribute('href')).toBe('/files/receipt-abc');
    expect(open[0]!.target).toBe('_blank');
    expect(open[0]!.rel).toContain('noopener');
    const downloads = [...el.querySelectorAll('a[download]')] as HTMLAnchorElement[];
    expect(downloads.length).toBe(1);
    expect(downloads[0]!.getAttribute('download')).toBe('TX-9_RECEIPT_FULL.html');
    expect(downloads[0]!.getAttribute('href')).toBe('/files/receipt-abc');
    expect(downloads[0]!.textContent).toContain('TX-9_RECEIPT_FULL.html');
    const frames = el.querySelectorAll('iframe');
    expect(frames.length).toBe(1);
    expect(frames[0]!.getAttribute('src')).toBe('/files/receipt-abc');
    expect(frames[0]!.hasAttribute('sandbox')).toBe(true);
    expect(frames[0]!.getAttribute('sandbox')).toBe('');
    expect(frames[0]!.getAttribute('title')).toBe('ใบเสร็จ TX-9');
  });

  it('a receipt image: เปิดดู ภาพใบเสร็จ, and no frame without a receipt', async () => {
    const { el, pending } = show(row({ id: 'TX-9' }));
    pending[0]!.resolve(documents('TX-9', [{ kind: 'receiptImage', path: '/files/img-1' }]));
    await settle();
    const open = linksNamed(el, 'เปิดดู ภาพใบเสร็จ');
    expect(open.length).toBe(1);
    expect(open[0]!.getAttribute('href')).toBe('/files/img-1');
    expect(el.querySelectorAll('a[download]').length).toBe(0);
    expect(el.querySelector('iframe')).toBeNull();
  });

  it('both kinds: a link each, the frame shows the receipt', async () => {
    const { el, pending } = show(row({ id: 'TX-9' }));
    pending[0]!.resolve(
      documents('TX-9', [
        { kind: 'receiptImage', path: '/files/img-1', download: 'TX-9_RECEIPT_IMAGE.png' },
        { kind: 'receipt', path: '/files/receipt-abc', download: 'TX-9_RECEIPT_FULL.html' },
      ]),
    );
    await settle();
    expect(linksNamed(el, 'เปิดดู ภาพใบเสร็จ').length).toBe(1);
    expect(linksNamed(el, 'เปิดดู ใบเสร็จ').length).toBe(1);
    expect(el.querySelectorAll('a[download]').length).toBe(2);
    const frames = el.querySelectorAll('iframe');
    expect(frames.length).toBe(1);
    expect(frames[0]!.getAttribute('src')).toBe('/files/receipt-abc');
  });

  it('a path not under /files/ is neither linked nor framed (slips.js:44, :53)', async () => {
    const { el, pending } = show(row({ id: 'TX-9' }));
    pending[0]!.resolve(documents('TX-9', [{ kind: 'receipt', path: 'javascript:alert(1)', download: 'x.html' }]));
    await settle();
    for (const a of el.querySelectorAll('a')) {
      expect(a.getAttribute('href') ?? '').not.toContain('javascript:');
    }
    expect(el.querySelector('iframe')).toBeNull();
  });

  it('no file: ยังไม่มีใบเสร็จที่บันทึกไว้สำหรับรายการนี้ (§9, slips.js:38)', async () => {
    const { el, pending } = show(row({ id: 'TX-9' }));
    pending[0]!.resolve(documents('TX-9', []));
    await settle();
    expect(el.textContent).toContain('ยังไม่มีใบเสร็จที่บันทึกไว้สำหรับรายการนี้');
    expect(el.textContent).not.toContain('กำลังโหลดใบเสร็จของ TX-9…');
    expect(el.querySelector('iframe')).toBeNull();
  });

  it('a refused read (signed out or not allowed): โหลดใบเสร็จไม่ได้ กรุณาเข้าสู่ระบบแล้วลองใหม่', async () => {
    for (const status of [401, 400, 403]) {
      const { el, pending } = show(row({ id: 'TX-9' }));
      pending[0]!.reject(new ApiError({ code: 'signIn', message: 'กรุณาเข้าสู่ระบบ', status }));
      await settle();
      expect(el.textContent, String(status)).toContain('โหลดใบเสร็จไม่ได้ กรุณาเข้าสู่ระบบแล้วลองใหม่');
      expect(el.textContent).not.toContain('กำลังโหลดใบเสร็จของ TX-9…');
      dispose?.();
      dispose = undefined;
      root.remove();
    }
  });

  it('closed before the documents arrive: nothing is drawn into it (§9, slips.js:39)', async () => {
    const { el, pending } = show(row({ id: 'TX-9' }));
    const sections = [...el.querySelectorAll('*')].filter(
      (n) => n.children.length === 0 && (n.textContent ?? '').includes('กำลังโหลดใบเสร็จของ TX-9…'),
    );
    expect(sections.length).toBe(1);
    const status = sections[0]!;
    const holder = status.parentElement!;
    dispose?.();
    dispose = undefined;
    pending[0]!.resolve(
      documents('TX-9', [{ kind: 'receipt', path: '/files/receipt-abc', download: 'TX-9_RECEIPT_FULL.html' }]),
    );
    await settle();
    expect(status.textContent).toBe('กำลังโหลดใบเสร็จของ TX-9…');
    expect(holder.querySelector('iframe')).toBeNull();
    expect(holder.querySelector('a')).toBeNull();
    expect(document.body.textContent).not.toContain('เอกสารที่บันทึกไว้ · TX-9');
  });

  // An unhandled rejection fails the vitest run on its own, so "nothing is thrown" needs no
  // listener here.
  it('closed before a refusal arrives: nothing is drawn and nothing is thrown', async () => {
    const { el, pending } = show(row({ id: 'TX-9' }));
    const before = el.innerHTML;
    dispose?.();
    dispose = undefined;
    pending[0]!.reject(new ApiError({ code: 'signIn', message: 'x', status: 401 }));
    await settle();
    expect(before).toContain('กำลังโหลดใบเสร็จของ TX-9…');
    expect(document.body.textContent).not.toContain('โหลดใบเสร็จไม่ได้ กรุณาเข้าสู่ระบบแล้วลองใหม่');
  });
});

describe('TransactionDetail — English (#476, W10 point 5)', () => {
  it('every label switches; the figures keep their digits', async () => {
    setLang('en');
    const { el, pending } = show(
      row({ customer: 'John Smith', status: 'COMPLETED', notes: 'regular', slipUrl: '/files/slip-1' }),
    );
    pending[0]!.resolve(
      documents('XC-B-2026-000042', [
        { kind: 'receipt', path: '/files/receipt-abc', download: 'XC-B-2026-000042_RECEIPT_FULL.html' },
      ]),
    );
    await settle();
    const text = el.textContent ?? '';
    expect(THAI.test(text), text).toBe(false);
    for (const attr of ['title', 'aria-label']) {
      for (const node of el.querySelectorAll(`[${attr}]`)) {
        expect(THAI.test(node.getAttribute(attr) ?? ''), `${attr}: ${node.getAttribute(attr)}`).toBe(false);
      }
    }
    expect(text).toContain('90,071,992,547,409.93');
    expect(text).toContain('3,195.55');
    expect(text).toContain('0.925926');
    expect(text).toContain('XC-B-2026-000042');
  });
});
