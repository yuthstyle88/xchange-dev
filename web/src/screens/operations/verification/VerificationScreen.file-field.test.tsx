// OL-7 row 101 file part (VERIFY.md on main 4a577dac, shot 056): the /operations/verification at-branch intake still
// shows the browser's English "Choose File / No file chosen". It uses the shared FileField part (shared/ui/FileField.tsx,
// tested in FileField.test.tsx); the input keeps name="file", its accept list and its change handler, so the intake and
// its reset (VerificationScreen.intake-file-reset.test.tsx) work as before. The harness is copied from that file.
// Written before the fix by claude/studio-claude-17. Do not edit, delete or weaken.
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang } from '../../../shared/i18n';
import { meOf } from '../../../test-support/me';
import { VerificationScreen } from './VerificationScreen';

const REVIEWER_SESSION = meOf({
  id: '00000000-0000-4000-8000-000000000001',
  role: 'DOCUMENTS',
  branch: 'MAIN',
  name: 'Reviewer Somchai',
  branchName: 'Head Office',
});

const ROUTE_BRANCH_CASES = '/api/operations/verification/branch-cases';

interface RecordedCall {
  method: string;
  pathname: string;
  headers: Headers;
  body: unknown;
}

type IntakeOutcome =
  | { kind: 'ok'; body: unknown; status?: number }
  | { kind: 'error'; status: number; body: unknown };

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

describe('VerificationScreen at-branch intake uses the shared FileField (OL-7 row 101)', () => {
  let calls: RecordedCall[] = [];
  let intakeOutcome: IntakeOutcome = { kind: 'ok', body: { caseId: 'case-101' }, status: 200 };
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setLang('th');
    calls = [];
    intakeOutcome = { kind: 'ok', body: { caseId: 'case-101' }, status: 200 };
    document.body.innerHTML = '';

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
      const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const url = new URL(rawUrl, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      const headers = new Headers(init?.headers);

      calls.push({
        method,
        pathname: url.pathname,
        headers,
        body: init?.body,
      });

      if (url.pathname === '/api/v3/views/me' || url.pathname === '/api/v3/views/staff_me') {
        return jsonResponse({ version: 1, changed: true, data: REVIEWER_SESSION });
      }

      if (url.pathname === '/api/v3/views/verification_policy') {
        return jsonResponse({
          version: 1,
          changed: true,
          data: {
            policyVersion: 1,
            branchReviewAllowed: true,
            acceptedDocumentTypes: ['PASSPORT', 'NATIONAL_ID'],
            documentsRequired: 1,
          },
        });
      }

      if (url.pathname === '/api/v3/views/verification_queue') {
        return jsonResponse({ version: 1, changed: true, data: [] });
      }

      if (url.pathname === ROUTE_BRANCH_CASES && method === 'POST') {
        if (intakeOutcome.kind === 'error') {
          return jsonResponse(intakeOutcome.body, intakeOutcome.status);
        }
        return jsonResponse(intakeOutcome.body, intakeOutcome.status ?? 200);
      }

      return jsonResponse({ error: { code: 'notFound', message: 'Not found' } }, 404);
    });
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  async function flush(): Promise<void> {
    for (let i = 0; i < 15; i++) {
      await new Promise((r) => setTimeout(r, 0));
    }
  }

  async function mountScreen(): Promise<void> {
    dispose?.();
    document.body.innerHTML = '';
    const host = document.createElement('div');
    document.body.appendChild(host);
    dispose = render(() => <VerificationScreen />, host);
    await flush();
  }

  function setCustomerId(id: string): void {
    const custInput = document.querySelector<HTMLInputElement>('input[name="customerId"]');
    expect(custInput).not.toBeNull();
    custInput!.value = id;
    custInput!.dispatchEvent(new Event('input', { bubbles: true }));
  }

  function chooseFile(file: File): HTMLInputElement {
    const fileInput = document.querySelector<HTMLInputElement>('input[name="file"]');
    expect(fileInput).not.toBeNull();
    let selectedFiles: File[] = [file];
    Object.defineProperty(fileInput!, 'files', {
      get: () => selectedFiles,
      configurable: true,
    });
    Object.defineProperty(fileInput!, 'value', {
      get: () => (selectedFiles.length > 0 ? `C:\\fakepath\\${selectedFiles[0]!.name}` : ''),
      set: (next: string) => {
        if (next === '') {
          selectedFiles = [];
        }
      },
      configurable: true,
    });
    fileInput!.dispatchEvent(new Event('change', { bubbles: true }));
    return fileInput!;
  }

  async function clickSubmit(): Promise<void> {
    const submitBtn = document.querySelector<HTMLElement>('[data-action="submit-branch-case"]');
    expect(submitBtn).not.toBeNull();
    submitBtn!.click();
    await flush();
  }

  const intakeCalls = () => calls.filter((c) => c.pathname === ROUTE_BRANCH_CASES && c.method === 'POST');

  const fieldName = () => document.querySelector('.xc-file-field .xc-file-field-name')?.textContent?.trim();

  it('VF-1 the intake file input sits in the shared part, inside its label, with the Thai pick text and empty name', async () => {
    await mountScreen();
    const input = document.querySelector<HTMLInputElement>('input[name="file"]');
    expect(input).not.toBeNull();
    expect(input!.type).toBe('file');
    expect(input!.accept).toBe('image/jpeg,image/png,application/pdf');
    expect(input!.classList.contains('xc-file-field-input')).toBe(true);
    expect(input!.closest('label > .xc-file-field')).not.toBeNull();
    expect(document.querySelector('.xc-file-field .xc-file-field-pick')?.textContent?.trim()).toBe('เลือกไฟล์');
    expect(fieldName()).toBe('ยังไม่ได้เลือกไฟล์');
  });

  it('VF-2 a chosen file shows its name; after a successful intake the name clears with the input', async () => {
    await mountScreen();
    setCustomerId('cust-A');
    const input = chooseFile(new File(['passport-a-bytes'], 'passport-a.jpg', { type: 'image/jpeg' }));
    await flush();
    expect(fieldName()).toBe('passport-a.jpg');
    await clickSubmit();
    expect(intakeCalls()).toHaveLength(1);
    expect(input.value).toBe('');
    expect(fieldName()).toBe('ยังไม่ได้เลือกไฟล์');
  });

  it('VF-3 the screen file no longer writes its own <input type="file">', async () => {
    const specifier = 'node:fs';
    const fs = (await import(/* @vite-ignore */ specifier)) as { readFileSync(f: string, e: 'utf8'): string };
    const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
    const src = fs.readFileSync(here + 'VerificationScreen.tsx', 'utf8');
    expect(src).not.toMatch(/type="file"/);
    expect(src).toMatch(/import \{ FileField \} from '\.\.\/\.\.\/\.\.\/shared\/ui\/FileField';/);
  });
});
