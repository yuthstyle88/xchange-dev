// V1-staff-guide (#397): v3's own staff guide at /staff-guide and /staff-guide.html, ported from
// xchange-v2-source@4d0bdbb:src/devices-public/staff-guide.html (ticket §10.1 G1–G10, §10.3 R1–R5) with its text in
// this screen's own table, web/src/screens/staff-guide/i18n.ts (design W6, W10).
// Written before the implementation by the lane controller (claude/7ca80c). Do not edit, delete or weaken.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { screenLookup, setLang } from '../../shared/i18n';

type StaffGuideScreen = (props: { print?: () => void }) => JSX.Element;

// Loaded by glob so that a missing file fails an assertion below instead of the whole file failing to import.
const screenModules = import.meta.glob<{ StaffGuide?: StaffGuideScreen }>('./StaffGuide.tsx', { eager: true });

function StaffGuide(): StaffGuideScreen {
  const component = screenModules['./StaffGuide.tsx']?.StaffGuide;
  expect(component, 'web/src/screens/staff-guide/StaffGuide.tsx exports function StaffGuide').toBeTypeOf('function');
  return component!;
}

const NS = 'staff-guide.';
const THAI = /[฀-๿]/;

// v2's text, verbatim (G1–G8b). The screen shows it through its table; these literals are what the table must hold.
const TITLE = 'คู่มือพนักงาน XC Desk v2';
const BACK = 'กลับแอดมิน';
const PRINT = 'พิมพ์คู่มือ';
const SECTIONS: readonly { heading: string; paragraphs: readonly string[] }[] = [
  {
    heading: '1. เข้าสู่ระบบและตรวจสาขา',
    paragraphs: [
      'ใช้บัญชีของตนเอง ตรวจชื่อพนักงานและสาขาก่อนทำงาน เจ้าของจัดการบัญชีและบทบาท Front, Backend, Manager, Rates, Monitor และ Finance ตามสิทธิ์ที่กำหนดไว้',
    ],
  },
  {
    heading: '2. เปิดกะและตรวจเรท',
    paragraphs: [
      'นับเงินจริงทุกสกุล แล้วบันทึกยอดเปิดกะตามที่นับได้ ตรวจเรทร้านรับซื้อและขายก่อนเปิดบริการ หน้าสกุลเงินแสดงเรทธนบัตรกสิกรไว้เทียบเฉพาะสกุลที่กสิกรมีเรท การเปิดสกุลเงินยังต้องกำหนดเรทร้านเอง ข้อมูล KB เป็นข้อมูลที่บันทึกไว้ ไม่ใช่ข้อมูลสด ให้ตรวจวันเวลาประกาศเสมอ',
    ],
  },
  {
    heading: '3. ลูกค้าเลือกแลกเงิน',
    paragraphs: [
      'เลือกทิศทาง สกุลเงิน และจำนวนจ่าย ระบบแสดงเฉพาะเรทของทิศทางนั้นและคำนวณยอดรับอัตโนมัติ ลูกค้าตรวจยอด รอ 3 วินาที แล้วยืนยันรับเลขคิว ยอดนี้เป็นยอดประมาณการก่อนค่าบริการ',
    ],
  },
  {
    heading: '4. รับคิวและตรวจพาสปอร์ต',
    paragraphs: [
      'เปิดหน้าคิวและรับเรื่องด้วยบัญชีของตนเอง เปิดหน้าทำรายการจากคิวเพื่อเติมข้อมูล ตรวจพาสปอร์ตและยืนยันด้วยตนเอง สถานะลูกค้าเปลี่ยนจาก Checking ID เป็น Pending หลังยืนยันตรวจเอกสาร ตรวจเรท จำนวนเงิน ค่าบริการ และยอดสุดท้ายกับลูกค้าอีกครั้งก่อนบันทึก',
    ],
  },
  {
    heading: '5. บันทึกและพิมพ์ใบเสร็จ',
    paragraphs: [
      'นับเงินก่อนส่งมอบ เมื่อบันทึกรายการสำเร็จจะแสดง Success พร้อมเลขรายการ พิมพ์ใบเสร็จกระดาษ 80 มม. ขนาด 100% ปิดหัวและท้ายกระดาษของเบราว์เซอร์ ใบเสร็จมีพื้นที่ท้ายเพิ่ม 20 มม. ตรวจยอด เงิน และใบเสร็จก่อนส่งลูกค้า',
      'ใบเสร็จและพาสปอร์ตผูกกับรหัสรายการ เปิดเอกสารจากรายละเอียดรายการ การทำงานกับเครื่องพิมพ์หรืออุปกรณ์จริงต้องตั้งค่าและทดสอบการเชื่อมต่อก่อน',
    ],
  },
  {
    heading: '6. ส่งกะและปิดกะ',
    paragraphs: [
      'นับเงิน เทียบยอดระบบ แยกส่วนต่างเงินสดออกจากกำไรและส่วนต่างตีราคา บันทึกเหตุผลของผลต่างและหลักฐานส่งมอบ ปิดกะและลงเวลาออกหลังส่งมอบเสร็จจริง ตัวเลขกำไรประมาณการไม่ใช่คำสั่งถอนเงินอัตโนมัติ',
    ],
  },
];

const READY = {
  status: 'ok',
  database: true,
  version: '1.0.0',
  build: '0123456789abcdef0123456789abcdef01234567',
  builtAt: '2026-10-08T01:02:03Z',
  channel: 'staging',
  ui: 'built',
};

let dispose: (() => void) | undefined;
let fetchMock: ReturnType<typeof vi.fn>;
const startUrl = window.location.href;

beforeEach(() => {
  window.localStorage.clear();
  setLang('th');
  fetchMock = vi.fn(async (input: unknown) => {
    if (String(input).endsWith('/health/ready')) {
      return new Response(JSON.stringify(READY), { status: 200 });
    }
    return new Response('{}', { status: 404 });
  });
  vi.stubGlobal('fetch', fetchMock);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  setLang('th');
  window.localStorage.clear();
  window.history.replaceState(null, '', startUrl);
});

function mount(props: { print?: () => void } = {}): HTMLElement {
  const Screen = StaffGuide();
  const host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => Screen(props), host);
  return host;
}

async function settle(): Promise<void> {
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}

/** The one key of this screen's collected table whose Thai text is `text`. */
function keyOf(text: string): string {
  const keys = Object.keys(screenLookup.th).filter((k) => k.startsWith(NS) && screenLookup.th[k] === text);
  expect(keys, `one key of web/src/screens/staff-guide/i18n.ts holds ${JSON.stringify(text)} in th`).toHaveLength(1);
  return keys[0]!;
}

/** What the screen must show for v2's Thai `text` in the given language. */
function shown(text: string, lang: 'th' | 'en'): string {
  return lang === 'th' ? text : screenLookup.en[keyOf(text)]!;
}

function backLink(host: HTMLElement, lang: 'th' | 'en'): HTMLAnchorElement {
  const links = Array.from(host.querySelectorAll('a')).filter((a) => a.textContent === shown(BACK, lang));
  expect(links, `one link reads ${JSON.stringify(shown(BACK, lang))}`).toHaveLength(1);
  return links[0]!;
}

function printButton(host: HTMLElement, lang: 'th' | 'en'): HTMLButtonElement {
  const buttons = Array.from(host.querySelectorAll('button')).filter((b) => b.textContent === shown(PRINT, lang));
  expect(buttons, `one button reads ${JSON.stringify(shown(PRINT, lang))}`).toHaveLength(1);
  return buttons[0]!;
}

function expectGuide(host: HTMLElement, lang: 'th' | 'en'): void {
  const h1s = Array.from(host.querySelectorAll('h1'));
  expect(h1s.map((h) => h.textContent)).toEqual([shown(TITLE, lang)]);

  const nav = backLink(host, lang).closest('nav');
  expect(nav, 'the back link is inside a <nav>').not.toBeNull();
  expect(nav!.contains(printButton(host, lang)), 'the print button is in the same <nav> as the back link').toBe(true);
  expect(nav!.textContent).toBe(`${shown(BACK, lang)} · ${shown(PRINT, lang)}`);

  const sections = Array.from(host.querySelectorAll('section'));
  expect(sections).toHaveLength(SECTIONS.length);
  SECTIONS.forEach((part, i) => {
    const section = sections[i]!;
    const headings = Array.from(section.querySelectorAll('h2')).map((h) => h.textContent);
    expect(headings, `section ${i + 1} heading`).toEqual([shown(part.heading, lang)]);
    const paragraphs = Array.from(section.querySelectorAll('p')).map((p) => p.textContent);
    expect(paragraphs, `section ${i + 1} paragraphs`).toEqual(part.paragraphs.map((p) => shown(p, lang)));
  });
}

describe('V1-staff-guide: the screen', () => {
  it('exists: StaffGuide.tsx exports StaffGuide and i18n.ts is collected under staff-guide.*', () => {
    expect(StaffGuide()).toBeTypeOf('function');
    expect(Object.keys(screenLookup.th).filter((k) => k.startsWith(NS)).length).toBeGreaterThan(0);
  });

  it('shows one h1, the v2 title (G1, G3)', () => {
    const host = mount();
    expect(Array.from(host.querySelectorAll('h1')).map((h) => h.textContent)).toEqual([TITLE]);
  });

  it('shows the nav line: link กลับแอดมิน to /, " · ", button พิมพ์คู่มือ, in one <nav> (G2)', () => {
    const host = mount();
    const link = backLink(host, 'th');
    expect(link.getAttribute('href')).toBe('/');
    const nav = link.closest('nav');
    expect(nav).not.toBeNull();
    expect(nav!.contains(printButton(host, 'th'))).toBe(true);
    expect(nav!.textContent).toBe(`${BACK} · ${PRINT}`);
    expect(link.compareDocumentPosition(printButton(host, 'th')) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it('shows the six parts of v2 in order, each a <section> with its <h2> and paragraphs, verbatim (G4–G8b)', () => {
    const host = mount();
    expectGuide(host, 'th');
  });

  it('takes every v2 text from its own table: each is one th entry under staff-guide.* (W10)', () => {
    const texts = [TITLE, BACK, PRINT, ...SECTIONS.flatMap((s) => [s.heading, ...s.paragraphs])];
    for (const text of texts) {
      const key = keyOf(text);
      const english = screenLookup.en[key];
      expect(english, `en entry for ${key}`).toBeTypeOf('string');
      expect(english!.length, `en entry for ${key} is not empty`).toBeGreaterThan(0);
      expect(THAI.test(english!), `en entry for ${key} has no Thai: ${JSON.stringify(english)}`).toBe(false);
    }
  });

  it('in English shows the en entry of each same key, in the same places, and no Thai at all', () => {
    setLang('en');
    const host = mount();
    expectGuide(host, 'en');
    expect(THAI.test(host.textContent ?? ''), `no Thai on the English screen: ${host.textContent}`).toBe(false);
  });

  it('a language switch while the guide is on screen changes its text in place', () => {
    const host = mount();
    expectGuide(host, 'th');
    setLang('en');
    expectGuide(host, 'en');
    setLang('th');
    expectGuide(host, 'th');
  });

  it('pressing พิมพ์คู่มือ calls print exactly once, and rendering alone does not (R3)', () => {
    const print = vi.fn();
    const host = mount({ print });
    expect(print).not.toHaveBeenCalled();
    printButton(host, 'th').click();
    expect(print).toHaveBeenCalledTimes(1);
  });

  it('without a print prop the button calls window.print', () => {
    const spy = vi.spyOn(window, 'print').mockImplementation(() => {});
    const host = mount();
    expect(spy).not.toHaveBeenCalled();
    printButton(host, 'th').click();
    expect(spy).toHaveBeenCalledTimes(1);
  });

  it('shows the build line from /health/ready in its footer (R4)', async () => {
    const host = mount();
    await settle();
    expect(host.querySelector('footer')?.textContent ?? '').toContain('v1.0.0 · 0123456 · staging');
  });

  it('calls no API of its own: the only request is /health/ready (G10, §8)', async () => {
    mount();
    await settle();
    const urls = fetchMock.mock.calls.map((call) => String((call as unknown[])[0]));
    expect(urls.length).toBeGreaterThan(0);
    for (const url of urls) expect(url).toMatch(/\/health\/ready$/);
  });

  it('ignores a query string: ?branch=X shows the same guide (§9)', async () => {
    window.history.replaceState(null, '', '/staff-guide');
    const plain = mount();
    await settle();
    const plainText = plain.textContent;
    dispose?.();
    dispose = undefined;
    document.body.innerHTML = '';

    window.history.replaceState(null, '', '/staff-guide?branch=X');
    const withQuery = mount();
    await settle();
    expectGuide(withQuery, 'th');
    expect(withQuery.textContent).toBe(plainText);
  });
});
