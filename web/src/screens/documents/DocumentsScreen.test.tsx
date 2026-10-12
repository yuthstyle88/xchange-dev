// #1349 V3-documents (design `docs/design/xchange-v3-wave3-report-documents.md` §A.6, §A.7, file **S** of §A.8):
// one screen, two audiences, for `/documents` (the view `documents`) and `/staff/documents` (the view
// `staff_documents`). Written before the implementation by the test-writer claude/5d45b017, on the FE lane
// controller's order (claude/5edf7bb5). Do not edit, delete or weaken; add tests in a file of your own.
//
// The contract these tests read [required]:
//   - `DocumentsScreen.tsx` exports `DocumentsScreen(props: { api: ApiClient; audience: Audience; reference: string;
//     branch: string; replace: (url: string) => void })`. It asks `api.view(audience's view name, { branch, filter: {
//     ref } })` once, with no `since` and no `limit`.
//   - `audiences.ts` exports `OWNER` (view `documents`) and `STAFF` (view `staff_documents`). **The staff audience
//     never sends a branch** (lane controller's ruling on #1349: a branch an employee names is the core's 403
//     `outOfScope`, and v2's staff page ignored it).
//   - One element `[data-kind="passport|paymentProof|receipt"]` per link, in the answer's order. Its label comes from
//     `documents.kind.*` (D-A7).
//   - The owner's item: `เปิดดู` (`target=_blank`, `rel=noopener`) and `ดาวน์โหลด <name>` (`download=<name>`), both
//     linking the answer's `href`. The back link is `/?branch=<encodeURIComponent(branch)>#transactions`.
//   - The staff item: a link to `href` named by the label, with no download. The back link is `/staff`.
//   - `/documents` refusals show their sentence where the list would be. `refusal.ts`'s `refusalText` turns a no-field
//     `400 invalidRequest` into `documents.badBranch` (§A.7). Every `/staff/documents` refusal calls `replace('/staff')`.
//   - Texts are `documents.*` in Thai and English, with no `zh` (L-A4), and no figure is parsed (no `Number(`).
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiError, type ApiClient, type ViewAnswer, type ViewParams } from '../../shared/api/client';
import { screenLookup, setLang } from '../../shared/i18n';

interface Link {
  kind: 'passport' | 'paymentProof' | 'receipt';
  href: string;
  download: string | null;
}
interface Data {
  ref: string;
  links: Link[];
}
interface AudienceLike {
  view: string;
}
interface ScreenModule {
  DocumentsScreen: (props: {
    api: ApiClient;
    audience: AudienceLike;
    reference: string;
    branch: string;
    replace: (url: string) => void;
  }) => JSX.Element;
}
interface AudiencesModule {
  OWNER: AudienceLike;
  STAFF: AudienceLike;
}
interface RefusalModule {
  refusalText: (err: ApiError, t: (key: string) => string) => string;
}
interface TextsModule {
  th?: Record<string, string>;
  en?: Record<string, string>;
  zh?: Record<string, string>;
}

const screens = import.meta.glob<ScreenModule>('./DocumentsScreen.tsx', { eager: true });
const audiencesFiles = import.meta.glob<AudiencesModule>('./audiences.ts', { eager: true });
const refusals = import.meta.glob<RefusalModule>('./refusal.ts', { eager: true });
const textFiles = import.meta.glob<TextsModule>('./i18n.ts', { eager: true });
const sources = import.meta.glob<string>('./*.{ts,tsx}', { query: '?raw', import: 'default', eager: true });

function screen(): ScreenModule['DocumentsScreen'] {
  const m = screens['./DocumentsScreen.tsx'];
  expect(m, 'web/src/screens/documents/DocumentsScreen.tsx exists').toBeDefined();
  return m!.DocumentsScreen;
}
function audiences(): AudiencesModule {
  const m = audiencesFiles['./audiences.ts'];
  expect(m, 'web/src/screens/documents/audiences.ts exists').toBeDefined();
  return m!;
}
function refusalText(): RefusalModule['refusalText'] {
  const m = refusals['./refusal.ts'];
  expect(m, 'web/src/screens/documents/refusal.ts exists').toBeDefined();
  return m!.refusalText;
}
function texts(): TextsModule {
  const m = textFiles['./i18n.ts'];
  expect(m, 'web/src/screens/documents/i18n.ts exists').toBeDefined();
  return m!;
}

// ── the fake ApiClient ──────────────────────────────────────────────────────

interface Asked {
  name: string;
  q: ViewParams<{ ref?: string }>;
}

let asked: Asked[];

function fakeApi(answer: Data | ApiError): ApiClient {
  return {
    async view<T>(name: string, q: ViewParams = {}): Promise<ViewAnswer<T>> {
      asked.push({ name, q: q as ViewParams<{ ref?: string }> });
      if (answer instanceof ApiError) throw answer;
      return { version: 3, changed: true, data: answer as T };
    },
    async page() {
      throw new Error('the documents screen pages nothing');
    },
    async act() {
      throw new Error('the documents screen writes nothing');
    },
  } as unknown as ApiClient;
}

function refusal(status: number, code: string, message: string, field?: string): ApiError {
  return new ApiError({ status, code, message, field });
}

const OWNER_DATA: Data = {
  ref: 'XC-20261006-000002',
  links: [
    { kind: 'passport', href: '/files/pass-key', download: 'XC-20261006-000002_PASSPORT.png' },
    { kind: 'paymentProof', href: '/files/slip-key', download: 'XC-20261006-000002_PAYMENT.pdf' },
    { kind: 'receipt', href: '/files/receipt-key', download: 'XC-20261006-000002_RECEIPT_FULL.html' },
  ],
};
const STAFF_DATA: Data = {
  ref: 'XC-20261006-000002',
  links: [
    { kind: 'passport', href: '/api/staff/file/pass-key', download: null },
    { kind: 'paymentProof', href: '/api/staff/file/slip-key', download: null },
    { kind: 'receipt', href: '/api/staff/file/receipt-key', download: null },
  ],
};

// ── mounting ─────────────────────────────────────────────────────────────────

let dispose: (() => void) | undefined;
let host: HTMLElement;

async function settle(): Promise<void> {
  for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
}

async function mount(
  audience: 'OWNER' | 'STAFF',
  answer: Data | ApiError,
  init: { reference?: string; branch?: string } = {},
): Promise<{ el: HTMLElement; replace: ReturnType<typeof vi.fn> }> {
  const Screen = screen();
  const replace = vi.fn();
  host = document.createElement('div');
  document.body.appendChild(host);
  const api = fakeApi(answer);
  const props = {
    api,
    audience: audiences()[audience],
    reference: init.reference ?? 'XC-20261006-000002',
    branch: init.branch ?? 'BR-2',
    replace,
  };
  dispose = render(() => <Screen {...props} />, host);
  await settle();
  return { el: host, replace };
}

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  asked = [];
  setLang('th');
});
asked = [];

function text(el: Element | null | undefined): string {
  return (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
}

function items(el: HTMLElement): HTMLElement[] {
  return [...el.querySelectorAll<HTMLElement>('[data-kind]')];
}

// ── what it asks ────────────────────────────────────────────────────────────

describe('V3-documents S: what the screen asks (§A.3, §A.7)', () => {
  it('the owner screen asks the view documents once, for the branch and the ref, with no since or limit', async () => {
    await mount('OWNER', OWNER_DATA, { reference: 'XC-1', branch: 'BR-2' });
    expect(asked.length).toBe(1);
    expect(asked[0]!.name).toBe('documents');
    expect(asked[0]!.q.branch).toBe('BR-2');
    expect(asked[0]!.q.filter).toEqual({ ref: 'XC-1' });
    expect(asked[0]!.q.since).toBeUndefined();
    expect(asked[0]!.q.limit).toBeUndefined();
  });

  it('the staff screen asks staff_documents for the ref and never sends a branch', async () => {
    await mount('STAFF', STAFF_DATA, { reference: 'XC-1', branch: 'BR-2' });
    expect(asked.length).toBe(1);
    expect(asked[0]!.name).toBe('staff_documents');
    expect(asked[0]!.q.filter).toEqual({ ref: 'XC-1' });
    expect(asked[0]!.q.branch === undefined || asked[0]!.q.branch === '', 'no branch for staff').toBe(true);
  });
});

// ── the lists ───────────────────────────────────────────────────────────────

describe("V3-documents S: the owner's list (§A.1 steps 3–7, 16–23)", () => {
  it('heading, the ref line, one item per file with its label, เปิดดู in a new tab and the download name', async () => {
    const { el } = await mount('OWNER', OWNER_DATA);
    expect(text(el)).toContain('เอกสารรายการ');
    expect(text(el)).toContain('XC-20261006-000002 · ใบเสร็จและพาสปอร์ตผูกกับรหัสออเดอร์เดียวกัน');
    expect(items(el).map((i) => i.dataset.kind)).toEqual(['passport', 'paymentProof', 'receipt']);
    const labels = ['พาสปอร์ต', 'หลักฐานชำระเงิน', 'ใบเสร็จ'];
    items(el).forEach((item, n) => {
      const link = OWNER_DATA.links[n]!;
      expect(text(item)).toContain(labels[n]!);
      const open = [...item.querySelectorAll('a')].find((a) => text(a) === 'เปิดดู');
      expect(open, `${link.kind}: เปิดดู`).toBeDefined();
      expect(open!.getAttribute('href')).toBe(link.href);
      expect(open!.getAttribute('target')).toBe('_blank');
      expect(open!.getAttribute('rel')).toBe('noopener');
      const down = item.querySelector('a[download]');
      expect(down, `${link.kind}: a download link`).not.toBeNull();
      expect(down!.getAttribute('download')).toBe(link.download);
      expect(down!.getAttribute('href')).toBe(link.href);
      expect(text(down)).toBe(`ดาวน์โหลด ${link.download}`);
    });
  });

  it('the back link is /?branch=<branch>#transactions, with the branch encoded', async () => {
    const { el } = await mount('OWNER', OWNER_DATA, { branch: 'B 2/x' });
    const back = [...el.querySelectorAll('a')].find((a) => text(a) === 'กลับรายการแลกเงิน');
    expect(back, 'กลับรายการแลกเงิน').toBeDefined();
    expect(back!.getAttribute('href')).toBe(`/?branch=${encodeURIComponent('B 2/x')}#transactions`);
  });
});

describe("V3-documents S: the staff list (§A.1 steps 28–32)", () => {
  it('the ref as heading, a link per file named by its label, no download, back to /staff', async () => {
    const { el, replace } = await mount('STAFF', STAFF_DATA);
    expect(text(el)).toContain('XC-20261006-000002');
    const labels = ['พาสปอร์ต', 'หลักฐานชำระเงิน', 'ใบเสร็จ'];
    expect(items(el).map((i) => i.dataset.kind)).toEqual(['passport', 'paymentProof', 'receipt']);
    items(el).forEach((item, n) => {
      const a = item.querySelector('a');
      expect(a, `${STAFF_DATA.links[n]!.kind}: a link`).not.toBeNull();
      expect(a!.getAttribute('href')).toBe(STAFF_DATA.links[n]!.href);
      expect(text(a)).toBe(labels[n]);
    });
    expect(el.querySelectorAll('a[download]').length, 'no download on the staff page').toBe(0);
    expect(text(el)).not.toContain('เปิดดู');
    const back = [...el.querySelectorAll('a')].find((a) => a.getAttribute('href') === '/staff');
    expect(back, 'a link back to /staff').toBeDefined();
    expect(text(back)).toBe('กลับหน้าพนักงาน');
    expect(replace).not.toHaveBeenCalled();
  });
});

// ── refusals ────────────────────────────────────────────────────────────────

describe('V3-documents S: a /documents refusal is a sentence on the page (D-A3, §A.7)', () => {
  it.each([
    ['not found', refusal(404, 'documentsNotFound', 'ไม่พบเอกสารรายการ', 'ref'), 'ไม่พบเอกสารรายการ'],
    ['no such branch', refusal(400, 'noSuchBranch', 'ไม่พบสาขาที่เลือก กรุณารีเฟรช', 'branch'), 'ไม่พบสาขาที่เลือก กรุณารีเฟรช'],
    ['a malformed branch (no-field invalidRequest)', refusal(400, 'invalidRequest', 'คำขอไม่ถูกต้อง'), 'สาขาไม่ถูกต้อง'],
    ['another branch named by an employee', refusal(403, 'outOfScope', 'สาขานี้อยู่นอกขอบเขตของบัญชีนี้'), 'สาขานี้อยู่นอกขอบเขตของบัญชีนี้'],
    ['the database is down', refusal(500, 'serverError', 'เซิร์ฟเวอร์ขัดข้อง'), 'เซิร์ฟเวอร์ขัดข้อง'],
  ])('%s → the sentence, no list, no redirect', async (_what, err, sentence) => {
    const { el, replace } = await mount('OWNER', err);
    expect(text(el)).toContain(sentence);
    expect(items(el).length).toBe(0);
    expect(replace).not.toHaveBeenCalled();
  });

  it("a field-carrying invalidRequest keeps the view's own sentence", async () => {
    const { el } = await mount('OWNER', refusal(400, 'invalidRequest', 'คำขอไม่ถูกต้อง', 'ref'));
    expect(text(el)).toContain('คำขอไม่ถูกต้อง');
    expect(text(el)).not.toContain('สาขาไม่ถูกต้อง');
  });
});

describe('V3-documents S: every /staff/documents refusal goes back to /staff (D-A4)', () => {
  it.each([
    ['nobody or the owner alone (401)', refusal(401, 'employeeAuthRequired', 'กรุณาเข้าสู่ระบบด้วยบัญชีพนักงาน')],
    ['RATES or SYSTEM (documentsNotReadable)', refusal(400, 'documentsNotReadable', 'บทบาทนี้ไม่มีสิทธิ์อ่านเอกสารลูกค้า')],
    ['a role the policy refuses (403)', refusal(403, 'roleNotAllowed', 'บทบาทนี้ไม่มีสิทธิ์ทำรายการ')],
    ['not found (404)', refusal(404, 'documentsNotFound', 'ไม่พบเอกสารรายการ', 'ref')],
  ])('%s → replace("/staff"), nothing listed', async (_what, err) => {
    const { el, replace } = await mount('STAFF', err);
    expect(replace).toHaveBeenCalledWith('/staff');
    expect(replace).toHaveBeenCalledTimes(1);
    expect(items(el).length).toBe(0);
  });
});

describe('V3-documents S: refusalText (§A.7, [required])', () => {
  const t = (key: string): string => `T(${key})`;

  it('a 400 invalidRequest with no field is documents.badBranch', () => {
    expect(refusalText()(refusal(400, 'invalidRequest', 'คำขอไม่ถูกต้อง'), t)).toBe('T(documents.badBranch)');
  });

  it("anything else is the refusal's own sentence", () => {
    const fn = refusalText();
    expect(fn(refusal(400, 'invalidRequest', 'คำขอไม่ถูกต้อง', 'ref'), t)).toBe('คำขอไม่ถูกต้อง');
    expect(fn(refusal(404, 'invalidRequest', 'x'), t)).toBe('x');
    expect(fn(refusal(400, 'noSuchBranch', 'ไม่พบสาขาที่เลือก กรุณารีเฟรช', 'branch'), t)).toBe(
      'ไม่พบสาขาที่เลือก กรุณารีเฟรช',
    );
    expect(fn(refusal(404, 'documentsNotFound', 'ไม่พบเอกสารรายการ', 'ref'), t)).toBe('ไม่พบเอกสารรายการ');
  });
});

// ── texts ───────────────────────────────────────────────────────────────────

describe('V3-documents S: TH and EN from documents.* (D-A7, L-A4)', () => {
  it("the kinds and the bad-branch sentence are v2's Thai words, with English twins and no zh", () => {
    const m = texts();
    expect(m.zh, 'no zh table (L-A4)').toBeUndefined();
    expect(Object.keys(m.en ?? {}).sort()).toEqual(Object.keys(m.th ?? {}).sort());
    expect(screenLookup.th['documents.kind.passport']).toBe('พาสปอร์ต');
    expect(screenLookup.th['documents.kind.paymentProof']).toBe('หลักฐานชำระเงิน');
    expect(screenLookup.th['documents.kind.receipt']).toBe('ใบเสร็จ');
    expect(screenLookup.th['documents.badBranch']).toBe('สาขาไม่ถูกต้อง');
    expect(screenLookup.en['documents.badBranch']).toBe('Invalid branch');
    for (const key of ['kind.passport', 'kind.paymentProof', 'kind.receipt']) {
      expect((screenLookup.en[`documents.${key}`] ?? '').trim(), `en documents.${key}`).not.toBe('');
    }
  });

  it('in English the labels are the en texts', async () => {
    setLang('en');
    const { el } = await mount('OWNER', OWNER_DATA);
    const labels = items(el).map((i) => text(i));
    expect(labels[0]).toContain(screenLookup.en['documents.kind.passport']!);
    expect(labels[1]).toContain(screenLookup.en['documents.kind.paymentProof']!);
    expect(labels[2]).toContain(screenLookup.en['documents.kind.receipt']!);
    expect(text(el)).not.toContain('พาสปอร์ต');
  });

  it('the bad-branch sentence follows the language', async () => {
    setLang('en');
    const { el } = await mount('OWNER', refusal(400, 'invalidRequest', 'คำขอไม่ถูกต้อง'));
    expect(text(el)).toContain('Invalid branch');
  });
});

describe('V3-documents S: the source', () => {
  it('parses no figure and names no role', () => {
    const own = Object.entries(sources).filter(([p]) => !p.includes('.test.'));
    expect(own.map(([p]) => p)).toContain('./DocumentsScreen.tsx');
    for (const [path, src] of own) {
      src.split('\n').forEach((line, i) => {
        expect(/Number\(|parseFloat|parseInt/.test(line), `${path}:${i + 1} ${line.trim()}`).toBe(false);
        expect(
          /['"`](OWNER|FRONT|MANAGER|RATES|SYSTEM|TELLER|EXECUTIVE|ADMIN|BRANCH_MANAGER)['"`]/.test(line),
          `${path}:${i + 1} ${line.trim()}`,
        ).toBe(false);
      });
    }
  });
});
