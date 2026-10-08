// V1-receipt-sample (#398): v3's own sample receipt at /receipt-sample, ported from
// xchange-v2-source@4d0bdbb:src/online/lib/receipt-html.ts:1 (ticket §10.1 R1–R14, §10.3 S1–S4), with its text in
// this screen's own table, web/src/screens/receipt-sample/i18n.ts (design W6, W7, W10).
// Written before the implementation by the lane controller (claude/7ca80c). Do not edit, delete or weaken.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { screenLookup, setLang } from '../../shared/i18n';
import logoUrl from './logo.png';
import qrUrl from './qr-telegram.png';

type ReceiptSampleScreen = (props: { print?: () => void }) => JSX.Element;

// Loaded by glob so that a missing file fails an assertion below instead of the whole file failing to import.
const screenModules = import.meta.glob<{ ReceiptSample?: ReceiptSampleScreen }>('./ReceiptSample.tsx', {
  eager: true,
});

function ReceiptSample(): ReceiptSampleScreen {
  const component = screenModules['./ReceiptSample.tsx']?.ReceiptSample;
  expect(component, 'web/src/screens/receipt-sample/ReceiptSample.tsx exports function ReceiptSample').toBeTypeOf(
    'function',
  );
  return component!;
}

const NS = 'receipt-sample.';
const THAI = /[฀-๿]/;
const BANNER = 'SAMPLE / ตัวอย่าง ไม่ใช่รายการจริง';

// v2's markup, verbatim from the source, with only the two data-URI `src`s and the two `onclick`s taken out: the
// images are checked against logo.png / qr-telegram.png below, and the print button is checked by clicking it.
const V2_HEADING = `<p class="screen-heading">XCHANGE / EXCHANGE TICKET</p>`;

const V2_RECEIPT = `<main class="receipt" aria-label="Exchange transaction receipt"><section class="main-ticket"><p style="font-size:8px;text-align:center;font-weight:bold;border:1px dashed;padding:3px;margin:1mm 0">SAMPLE / ตัวอย่าง ไม่ใช่รายการจริง</p><header class="brand"><div class="logo-window"><img class="logo" alt="XCHANGE Currency Exchange"></div><div class="ticket-label"><strong>EXCHANGE</strong>TRANSACTION<br>RECEIPT / 01</div></header><div class="route" aria-hidden="true"><span class="route-code">USD</span><div class="route-mid"><svg viewBox="0 0 100 20" fill="none"><path d="M0 10H91M85 4L93 10L85 16" stroke="currentColor" stroke-width="1" stroke-dasharray="3 3"/></svg>CURRENCY EXCHANGE</div><span class="route-code">THB</span></div><dl><div class="row receipt-number"><dt>Receipt No.</dt><dd>EX202609270001</dd></div><div class="row"><dt>Type</dt><dd>SELL</dd></div><div class="row passenger"><dt>Customer</dt><dd>John Smith</dd></div><div class="row"><dt>Currency</dt><dd>USD</dd></div><div class="row"><dt>Amount</dt><dd>1,000.00 USD</dd></div><div class="row"><dt>Booked Rate</dt><dd>33.2500</dd></div><div class="row"><dt>Time</dt><dd>27/09/2026 14:05</dd></div><div class="row"><dt>Customer ID</dt><dd>CX-20260927-00001</dd></div><div class="row"><dt>Pay Via</dt><dd>Cash</dd></div><div class="row"><dt>Ref No.</dt><dd>—</dd></div><div class="row"><dt>Staff</dt><dd>ADMIN01</dd></div></dl><dl id="settlement"><div class="row"><dt>Customer Fee THB</dt><dd>0.00</dd></div><div class="row"><dt>Net Received THB</dt><dd>33,250.00</dd></div></dl><dl><div class="row total"><dt>THB Total</dt><dd>33,250.00<span class="unit"> THB</span></dd></div></dl></section><div class="perforation" aria-hidden="true"><i class="notch left"></i><span>KEEP EXPLORING</span><i class="notch right"></i></div>
<footer class="stub"><h2 class="promo">USDT REAL-TIME RATE UPDATE<br>&amp; RATE RESERVATION</h2><div class="footer-grid"><a href="https://t.me/XCHANGEXTK" aria-label="Open Telegram"><img class="qr" alt="Scan to open Telegram @XCHANGEXTK"></a><div><div class="scan-copy"><b>Scan QR Code for</b><br>Live Exchange Rates<br>&amp; Rate Reservation</div><div class="contacts">WhatsApp<br><a href="https://wa.me/66968799989"><strong>(+66) 96 879 9989</strong></a><br>Telegram: <a href="https://t.me/XCHANGEXTK"><strong>@XCHANGEXTK</strong></a></div></div></div><div class="company"><strong>T.K. EXCHANGE MONEY CO., LTD.</strong>REG. 0205567035266 &middot; LIC. MC125670033<br>HEAD OFFICE: 75/223 MOO 12, SOI JOMTIEN 7<br>NONG PRUE, BANG LAMUNG, CHON BURI, THAILAND</div></footer></main>`;

const V2_PRINT_BUTTON = `<button class="print-button">Print receipt &middot; 80 × 90 mm</button>`;

const V2_PRINT_TIP = `<p class="print-tip">NEOCAL &middot; 80 × 90 mm paper &middot; Scale 100%<br>Turn off browser headers and footers.</p>`;

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
  const Screen = ReceiptSample();
  const host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => Screen(props), host);
  return host;
}

async function settle(): Promise<void> {
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}

function parse(html: string): Element {
  const template = document.createElement('template');
  template.innerHTML = html;
  return template.content.firstElementChild!;
}

/** Children that render: comments (Solid's markers) and whitespace-only text are dropped, adjacent text is merged. */
function significant(node: Node): Node[] {
  const out: Node[] = [];
  for (const child of Array.from(node.childNodes)) {
    if (child.nodeType === Node.COMMENT_NODE) continue;
    if (child.nodeType === Node.TEXT_NODE) {
      const last = out[out.length - 1];
      if (last && last.nodeType === Node.TEXT_NODE) {
        out[out.length - 1] = document.createTextNode((last as Text).data + (child as Text).data);
      } else {
        out.push(document.createTextNode((child as Text).data));
      }
      continue;
    }
    out.push(child);
  }
  return out.filter((n) => n.nodeType !== Node.TEXT_NODE || (n as Text).data.trim() !== '');
}

// Attributes that carry text or meaning: the actual element may not add one that v2's element does not have.
const MEANINGFUL = ['aria-label', 'aria-hidden', 'alt', 'title', 'href', 'id', 'role', 'style'];

function expectSameTree(actual: Node, expected: Node, path: string): void {
  if (expected.nodeType === Node.TEXT_NODE) {
    expect(actual.nodeType, `${path}: a text node`).toBe(Node.TEXT_NODE);
    expect((actual as Text).data, `${path}: text`).toBe((expected as Text).data);
    return;
  }
  const want = expected as Element;
  expect(actual.nodeType, `${path}: an element <${want.localName}>`).toBe(Node.ELEMENT_NODE);
  const got = actual as Element;
  const here = `${path} > ${want.localName}${want.getAttribute('class') ? '.' + want.classList.value.replace(/ /g, '.') : ''}`;
  expect(got.localName, `${here}: tag`).toBe(want.localName);
  expect(Array.from(got.classList).sort(), `${here}: classes`).toEqual(Array.from(want.classList).sort());
  for (const attr of Array.from(want.attributes)) {
    if (attr.name === 'class') continue;
    if (attr.name === 'style') {
      expect((got as HTMLElement).style.cssText, `${here}: style`).toBe((want as HTMLElement).style.cssText);
      continue;
    }
    expect(got.getAttribute(attr.name), `${here}: @${attr.name}`).toBe(attr.value);
  }
  for (const name of MEANINGFUL) {
    if (!want.hasAttribute(name)) expect(got.hasAttribute(name), `${here}: no @${name} that v2 lacks`).toBe(false);
  }
  const gotKids = significant(got);
  const wantKids = significant(want);
  expect(
    gotKids.map((n) => (n.nodeType === Node.TEXT_NODE ? '#text' : (n as Element).localName)),
    `${here}: children`,
  ).toEqual(wantKids.map((n) => (n.nodeType === Node.TEXT_NODE ? '#text' : (n as Element).localName)));
  wantKids.forEach((kid, i) => expectSameTree(gotKids[i]!, kid, here));
}

function only(host: HTMLElement, selector: string): Element {
  const found = Array.from(host.querySelectorAll(selector));
  expect(found, `exactly one ${selector}`).toHaveLength(1);
  return found[0]!;
}

function printButton(host: HTMLElement): HTMLButtonElement {
  return only(host, 'button.print-button') as HTMLButtonElement;
}

function banner(host: HTMLElement): Element {
  const first = only(host, 'main.receipt > section.main-ticket').firstElementChild;
  expect(first?.localName, 'the banner is the first element of section.main-ticket').toBe('p');
  return first!;
}

describe('V1-receipt-sample: the screen', () => {
  it('exists: ReceiptSample.tsx exports ReceiptSample and i18n.ts is collected under receipt-sample.*', () => {
    expect(ReceiptSample()).toBeTypeOf('function');
    expect(Object.keys(screenLookup.th).filter((k) => k.startsWith(NS)).length).toBeGreaterThan(0);
  });

  it('shows the screen heading as v2 does (R2)', () => {
    const host = mount();
    expectSameTree(only(host, 'p.screen-heading'), parse(V2_HEADING), 'heading');
  });

  it("shows v2's receipt, element for element, text for text, class for class (R3–R12, S1)", () => {
    const host = mount();
    expectSameTree(only(host, 'main.receipt'), parse(V2_RECEIPT), 'receipt');
  });

  it('shows the print button and the print tip as v2 does (R13)', () => {
    const host = mount();
    expectSameTree(printButton(host), parse(V2_PRINT_BUTTON), 'button');
    expectSameTree(only(host, 'p.print-tip'), parse(V2_PRINT_TIP), 'tip');
  });

  it('keeps v2\'s order: heading, receipt, print button, print tip', () => {
    const host = mount();
    const order = ['p.screen-heading', 'main.receipt', 'button.print-button', 'p.print-tip'].map((s) => only(host, s));
    for (let i = 1; i < order.length; i += 1) {
      const before = order[i - 1]!;
      expect(before.compareDocumentPosition(order[i]!) & Node.DOCUMENT_POSITION_FOLLOWING, `${i}`).toBeTruthy();
      expect(before.contains(order[i]!), `${i}: not nested`).toBe(false);
    }
  });

  it('shows the logo and the Telegram QR from logo.png and qr-telegram.png (R5, R11)', () => {
    const host = mount();
    expect(only(host, 'main.receipt img.logo').getAttribute('src')).toBe(logoUrl);
    expect(only(host, 'main.receipt img.qr').getAttribute('src')).toBe(qrUrl);
  });

  it('pressing the print button calls print exactly once, and rendering alone does not (§9)', () => {
    const print = vi.fn();
    const host = mount({ print });
    expect(print).not.toHaveBeenCalled();
    printButton(host).click();
    expect(print).toHaveBeenCalledTimes(1);
  });

  it('without a print prop the button calls window.print', () => {
    const spy = vi.spyOn(window, 'print').mockImplementation(() => {});
    const host = mount();
    expect(spy).not.toHaveBeenCalled();
    printButton(host).click();
    expect(spy).toHaveBeenCalledTimes(1);
  });

  it('calls no API of its own: any request it makes is /health/ready (R14, §8)', async () => {
    mount();
    await settle();
    // AppShell's account bar reads the shared session's `me` view (#402); that request is the
    // shell's, not this screen's, and whether this test sees it depends on test order.
    const own = fetchMock.mock.calls
      .map((call) => String((call as unknown[])[0]))
      .filter((url) => !/\/api\/v3\/views\/me(\?|$)/.test(url));
    for (const url of own) expect(url).toMatch(/\/health\/ready$/);
  });

  it('never shows the branch name (§9)', async () => {
    window.history.replaceState(null, '', '/receipt-sample?branch=ZZBRANCHQ');
    const host = mount();
    await settle();
    expect(host.textContent ?? '').not.toContain('ZZBRANCHQ');
    expect(host.innerHTML).not.toContain('ZZBRANCHQ');
  });

  it('takes the Thai banner from its own table: one th entry under receipt-sample.* (W10)', () => {
    const keys = Object.keys(screenLookup.th).filter((k) => k.startsWith(NS) && screenLookup.th[k] === BANNER);
    expect(keys, `one key of web/src/screens/receipt-sample/i18n.ts holds ${JSON.stringify(BANNER)} in th`).toHaveLength(1);
    const english = screenLookup.en[keys[0]!];
    expect(english, 'its en entry').toBeTypeOf('string');
    expect(english!.length).toBeGreaterThan(0);
    expect(THAI.test(english!), `its en entry has no Thai: ${JSON.stringify(english)}`).toBe(false);
  });

  it('in English the banner is the en entry of the same key and no Thai is left on the screen', () => {
    const key = Object.keys(screenLookup.th).find((k) => k.startsWith(NS) && screenLookup.th[k] === BANNER);
    expect(key, 'the banner key').toBeTypeOf('string');
    setLang('en');
    const host = mount();
    expect(banner(host).textContent).toBe(screenLookup.en[key!]);
    expect(THAI.test(host.textContent ?? ''), `no Thai on the English screen: ${host.textContent}`).toBe(false);
  });

  it('a language switch while the sample is on screen changes the banner in place', () => {
    const key = Object.keys(screenLookup.th).find((k) => k.startsWith(NS) && screenLookup.th[k] === BANNER);
    expect(key, 'the banner key').toBeTypeOf('string');
    const host = mount();
    expect(banner(host).textContent).toBe(BANNER);
    setLang('en');
    expect(banner(host).textContent).toBe(screenLookup.en[key!]);
    setLang('th');
    expect(banner(host).textContent).toBe(BANNER);
  });
});
