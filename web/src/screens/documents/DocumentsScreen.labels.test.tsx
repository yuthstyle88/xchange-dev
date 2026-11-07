// OL-6 docs-labels: the owner's answers 1B (web half) and 2A on #564 (issuecomment-6705475792, 2026-11-07, on the
// questions in issuecomment-6705285102), shots 017 and 052.
// - 2A: the staff page's heading was the bare ref (`XC-20261105-000023`). It keeps the ref and gets a label: one key
//   `refTitle`, th "เอกสารของรายการ {ref}", en "Documents for {ref}".
// - 1B web half: the owner's download link read "ดาวน์โหลด <stored name>" (a `3f9c….bin` on xc-v3). It now reads the
//   document's kind (`link.kind`, set by the server at api/src/presentation/http/v3/views/documents.rs:110-112) and the
//   ref: "ดาวน์โหลด พาสปอร์ต XC-…". The `download` and `href` attributes stay exactly as the server sends them (the api
//   ticket of 1B renames the stored file).
// Written before the fix by claude/bbe9ce8b for the OL-6 sub-controller claude/1cc51615. Do not edit, delete or weaken.
import { afterEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import type { ApiClient } from '../../shared/api/client';
import { setLang } from '../../shared/i18n';
import { makeDocumentsData, makeDocumentsLink } from '../../shared/testing/fixtures';
import { OWNER, STAFF, type Audience } from './audiences';
import { DocumentsScreen } from './DocumentsScreen';
import { en, th } from './i18n';

const REF = 'XC-20261105-000023';

const OWNER_DATA = makeDocumentsData({
  ref: REF,
  links: [
    makeDocumentsLink({ kind: 'passport', href: '/files/3f9c0a.bin', download: '3f9c0a1b2c3d.bin' }),
    makeDocumentsLink({ kind: 'paymentProof', href: '/files/77aa.bin', download: '77aa88bb.bin' }),
    makeDocumentsLink({ kind: 'receipt', href: '/files/r-1', download: 'XC-20261105-000023_RECEIPT_FULL.html' }),
  ],
});

const STAFF_DATA = makeDocumentsData({
  ref: REF,
  links: [makeDocumentsLink({ kind: 'passport', href: '/api/staff/file/p-1', download: null })],
});

let dispose: (() => void) | undefined;
let host: HTMLElement | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  host = undefined;
  setLang('th');
});

async function mount(audience: Audience, data: ReturnType<typeof makeDocumentsData>): Promise<HTMLElement> {
  const api = { view: async () => ({ version: 1, changed: true, data }) } as unknown as ApiClient;
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(
    () => <DocumentsScreen api={api} audience={audience} reference={REF} branch="BR-2" replace={() => undefined} />,
    host,
  );
  for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
  return host;
}

const text = (el: Element | null | undefined) => (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
const downloads = (el: ParentNode) => [...el.querySelectorAll<HTMLAnchorElement>('a[download]')];

describe('docs-labels 2A: the staff heading labels the ref', () => {
  it('DL-1 th: the heading reads เอกสารของรายการ <ref>', async () => {
    const el = await mount(STAFF, STAFF_DATA);
    expect(text(el.querySelector('h2.documents-title'))).toBe(`เอกสารของรายการ ${REF}`);
  });

  it('DL-2 en: the heading reads Documents for <ref>', async () => {
    setLang('en');
    const el = await mount(STAFF, STAFF_DATA);
    expect(text(el.querySelector('h2.documents-title'))).toBe(`Documents for ${REF}`);
  });

  it('DL-3 the key is refTitle in both tables', () => {
    expect((th as Record<string, string>).refTitle).toBe('เอกสารของรายการ {ref}');
    expect((en as Record<string, string>).refTitle).toBe('Documents for {ref}');
  });
});

describe("docs-labels 1B web half: the owner's download links read the kind and the ref", () => {
  it('DL-4 th: each link reads ดาวน์โหลด <kind label> <ref>, in the answer order', async () => {
    const el = await mount(OWNER, OWNER_DATA);
    expect(downloads(el).map(text)).toEqual([
      `ดาวน์โหลด พาสปอร์ต ${REF}`,
      `ดาวน์โหลด หลักฐานชำระเงิน ${REF}`,
      `ดาวน์โหลด ใบเสร็จ ${REF}`,
    ]);
  });

  it('DL-5 en: Download <kind label> <ref>', async () => {
    setLang('en');
    const el = await mount(OWNER, OWNER_DATA);
    expect(downloads(el).map(text)).toEqual([
      `Download Passport ${REF}`,
      `Download Payment proof ${REF}`,
      `Download Receipt ${REF}`,
    ]);
  });

  it('DL-6 no stored file name shows in the page text', async () => {
    const el = await mount(OWNER, OWNER_DATA);
    expect(text(el)).not.toContain('.bin');
    expect(text(el)).not.toContain('_RECEIPT_FULL.html');
  });

  it('DL-7 download and href stay exactly as the server sends them', async () => {
    const el = await mount(OWNER, OWNER_DATA);
    downloads(el).forEach((a, n) => {
      expect(a.getAttribute('download')).toBe(OWNER_DATA.links[n]!.download);
      expect(a.getAttribute('href')).toBe(OWNER_DATA.links[n]!.href);
    });
  });
});
