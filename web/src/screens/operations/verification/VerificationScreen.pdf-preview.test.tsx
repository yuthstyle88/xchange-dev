// #2580: Verification document preview must render an <iframe> (not <img>) when the document's
// mime is application/pdf, and keep rendering <img> for image/png and image/jpeg documents.
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang } from '../../../shared/i18n';
import { meOf } from '../../../test-support/me';
import { VerificationScreen } from './VerificationScreen';

const CASE_ID = '11111111-1111-1111-1111-111111111111';
const DOC_PDF_ID = 'dddddddd-0001-0001-0001-000000000001';
const DOC_PNG_ID = 'dddddddd-0002-0002-0002-000000000002';
const DOC_JPEG_ID = 'dddddddd-0003-0003-0003-000000000003';

const REVIEWER_SESSION = meOf({
  id: '00000000-0000-4000-8000-000000000001',
  role: 'VERIFICATION_REVIEWER',
  branch: 'MAIN',
  name: 'Reviewer Somchai',
  branchName: 'Head Office',
});

const QUEUE_ITEMS = [
  {
    id: CASE_ID,
    status: 'UNDER_REVIEW',
    submittedAt: '2026-10-16T06:00:00Z',
    customerDisplayName: 'Somchai Jaidee',
    identityLast4: '1234',
    branchId: 'MAIN',
  },
];

const CASE_DATA = {
  case: {
    id: CASE_ID,
    status: 'UNDER_REVIEW',
    version: 2,
    submittedAt: '2026-10-16T06:00:00Z',
    customerDisplayName: 'Somchai Jaidee',
    identityLast4: '1234',
    branchId: 'MAIN',
  },
  documents: [
    {
      id: DOC_PDF_ID,
      documentType: 'PASSPORT',
      mime: 'application/pdf',
      sizeBytes: 4096,
      uploadedAt: '2026-10-16T06:00:00Z',
    },
    {
      id: DOC_PNG_ID,
      documentType: 'NATIONAL_ID',
      mime: 'image/png',
      sizeBytes: 1024,
      uploadedAt: '2026-10-16T06:01:00Z',
    },
    {
      id: DOC_JPEG_ID,
      documentType: 'PASSPORT',
      mime: 'image/jpeg',
      sizeBytes: 2048,
      uploadedAt: '2026-10-16T06:02:00Z',
    },
  ],
  audit: [],
};

function jsonResponse(data: unknown, status = 200): Response {
  return new Response(JSON.stringify(data), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

describe('#2580 VerificationScreen PDF and image document preview', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | null = null;

  async function settle(): Promise<void> {
    for (let i = 0; i < 15; i += 1) {
      await vi.advanceTimersByTimeAsync(0);
    }
  }

  beforeEach(() => {
    vi.useFakeTimers();
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);

    vi.stubGlobal('fetch', async (input: RequestInfo | URL) => {
      const rawUrl =
        typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const parsed = new URL(rawUrl, 'http://localhost');

      if (parsed.pathname === '/api/v3/views/me' || parsed.pathname === '/api/v3/views/staff_me') {
        return jsonResponse({ version: 1, changed: true, data: REVIEWER_SESSION });
      }
      if (parsed.pathname === '/api/v3/views/verification_policy') {
        return jsonResponse({
          version: 1,
          changed: true,
          data: { branchReviewAllowed: false },
        });
      }
      if (parsed.pathname === '/api/v3/views/verification_queue') {
        return jsonResponse({
          version: 1,
          changed: true,
          data: QUEUE_ITEMS,
        });
      }
      if (parsed.pathname === '/api/v3/views/verification_case') {
        return jsonResponse({
          version: CASE_DATA.case.version,
          changed: true,
          data: CASE_DATA,
        });
      }
      if (parsed.pathname.includes('/documents/') && parsed.pathname.endsWith('/link')) {
        const docId = parsed.pathname.split('/documents/')[1]?.split('/link')[0] ?? '';
        return jsonResponse({
          url: `/api/verification/content/mock-token-${docId}`,
          expiresAt: '2030-01-01T00:00:00Z',
        });
      }
      if (parsed.pathname.startsWith('/api/verification/content/')) {
        const isPdf = parsed.pathname.endsWith(DOC_PDF_ID);
        return new Response(new Uint8Array([37, 80, 68, 70]), {
          status: 200,
          headers: { 'Content-Type': isPdf ? 'application/pdf' : 'image/png' },
        });
      }
      return jsonResponse({ error: { code: 'notFound', message: 'Not found' } }, 404);
    });
  });

  afterEach(() => {
    if (dispose) {
      dispose();
      dispose = null;
    }
    host.remove();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    vi.useRealTimers();
  });

  async function mountAndSelectCase(): Promise<void> {
    dispose = render(() => <VerificationScreen />, host);
    await settle();
    const caseRow = host.querySelector<HTMLElement>(`[data-case-id="${CASE_ID}"]`);
    expect(caseRow).not.toBeNull();
    caseRow!.click();
    await settle();
  }

  it('row 1: Preview a document with mime application/pdf -> An iframe with the content URL is rendered, no img', async () => {
    await mountAndSelectCase();

    const pdfBtn = host.querySelector<HTMLButtonElement>(`[data-doc-id="${DOC_PDF_ID}"]`);
    expect(pdfBtn).not.toBeNull();
    pdfBtn!.click();
    await settle();

    const preview = host.querySelector('.xc-verification-doc-preview');
    expect(preview).not.toBeNull();
    const iframe = preview!.querySelector<HTMLIFrameElement>('iframe');
    expect(iframe).not.toBeNull();
    expect(iframe!.getAttribute('src')).toBe(`/api/verification/content/mock-token-${DOC_PDF_ID}`);
    expect(preview!.querySelector('img')).toBeNull();
  });

  it('row 2 & edge 2: Preview an image/png or image/jpeg document after a PDF -> Switching from PDF to image renders <img> with same src and removes <iframe>', async () => {
    await mountAndSelectCase();

    // Open PDF first -> renders iframe
    host.querySelector<HTMLButtonElement>(`[data-doc-id="${DOC_PDF_ID}"]`)!.click();
    await settle();
    expect(host.querySelector('.xc-verification-doc-preview iframe')).not.toBeNull();
    expect(host.querySelector('.xc-verification-doc-preview img')).toBeNull();

    // Switch to image/png -> renders img with content URL, no iframe
    host.querySelector<HTMLButtonElement>(`[data-doc-id="${DOC_PNG_ID}"]`)!.click();
    await settle();
    const pngImg = host.querySelector<HTMLImageElement>('.xc-verification-doc-preview img');
    expect(pngImg).not.toBeNull();
    expect(pngImg!.getAttribute('src')).toBe(`/api/verification/content/mock-token-${DOC_PNG_ID}`);
    expect(host.querySelector('.xc-verification-doc-preview iframe')).toBeNull();

    // Switch to image/jpeg -> renders img with content URL, no iframe
    host.querySelector<HTMLButtonElement>(`[data-doc-id="${DOC_JPEG_ID}"]`)!.click();
    await settle();
    const jpegImg = host.querySelector<HTMLImageElement>('.xc-verification-doc-preview img');
    expect(jpegImg).not.toBeNull();
    expect(jpegImg!.getAttribute('src')).toBe(
      `/api/verification/content/mock-token-${DOC_JPEG_ID}`,
    );
    expect(host.querySelector('.xc-verification-doc-preview iframe')).toBeNull();
  });

  it('edge 1: PDF iframe src uses /api/verification/content/... URL and never points to /files/', async () => {
    await mountAndSelectCase();

    host.querySelector<HTMLButtonElement>(`[data-doc-id="${DOC_PDF_ID}"]`)!.click();
    await settle();

    const iframe = host.querySelector<HTMLIFrameElement>('.xc-verification-doc-preview iframe');
    expect(iframe).not.toBeNull();
    const src = iframe!.getAttribute('src') ?? '';
    expect(src).toContain('/api/verification/content/');
    expect(src).not.toContain('/files/');
  });
});
