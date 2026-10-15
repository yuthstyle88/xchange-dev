import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiError, type ApiClient, type ViewAnswer, type ViewParams } from '../../shared/api/client';
import type { DocumentsData } from '../../shared/api/generated/documents';
import { screenLookup, setLang, translator } from '../../shared/i18n';
import { th } from '../../shared/i18n/th';
import { en } from '../../shared/i18n/en';
import { th as screenTh, en as screenEn } from './i18n';
import { OWNER } from './audiences';
import { DocumentsScreen } from './DocumentsScreen';
import { refusalText } from './refusal';

const tr = translator(screenLookup);

function refusal(status: number, code: string, message: string, field?: string): ApiError {
  return new ApiError({ status, code, message, field });
}

function fakeApi(answer: DocumentsData | ApiError | Error): ApiClient {
  return {
    async view<T>(_name: string, _q: ViewParams = {}): Promise<ViewAnswer<T>> {
      if (answer instanceof Error) throw answer;
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

describe('WE-35: screens/documents/ — wordedError handling (#1784)', () => {
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

  async function settle(): Promise<void> {
    for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
  }

  async function mount(answer: DocumentsData | ApiError | Error) {
    const replace = vi.fn();
    const api = fakeApi(answer);
    const props = {
      api,
      audience: OWNER,
      reference: 'XC-20261015-000001',
      branch: 'BR-1',
      replace,
    };
    dispose = render(() => <DocumentsScreen {...props} />, host);
    await settle();
    return { el: host, replace };
  }

  function getRefusalText(): string | null {
    const el = host.querySelector('.documents-refusal');
    return el ? (el.textContent ?? '').trim() : null;
  }

  // ──────────────────────────────────────────────────────────────────────────
  // Catch 1: onMount view failure
  // ──────────────────────────────────────────────────────────────────────────
  describe('Catch 1: onMount documents view failure', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      await mount(new TypeError('Failed to fetch'));
      expect(getRefusalText()).toBe(th['error.network']);
      expect(getRefusalText()).not.toContain('Failed to fetch');

      // In EN
      dispose?.();
      dispose = undefined;
      host.innerHTML = '';
      setLang('en');

      await mount(new TypeError('Failed to fetch'));
      expect(getRefusalText()).toBe(en['error.network']);
      expect(getRefusalText()).not.toContain('Failed to fetch');
    });

    it('(2) 500 refusal shows error.serverError in TH', async () => {
      await mount(refusal(500, 'roleNotAllowedHere', 'Internal DB fault'));
      expect(getRefusalText()).toBe(th['error.serverError']);
      expect(getRefusalText()).not.toContain('Internal DB fault');
      expect(getRefusalText()).not.toContain(th['error.roleNotAllowedHere']);
    });

    it('(3) refusal with unkeyed code and server message shows that message', async () => {
      const serverMsg = 'ข้อความปฏิเสธเอกสารจากเซิร์ฟเวอร์เฉพาะ';
      await mount(refusal(400, 'unkeyedDocRefusal', serverMsg));
      expect(getRefusalText()).toBe(serverMsg);
      expect(getRefusalText()).not.toBe(th['error.serverError']);
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // WE4: Screen-specific handling (documents.badBranch)
  // ──────────────────────────────────────────────────────────────────────────
  describe('WE4: Screen-specific sentence for no-field 400 invalidRequest', () => {
    it('no-field 400 invalidRequest shows badBranch in TH and EN', async () => {
      await mount(refusal(400, 'invalidRequest', 'คำขอไม่ถูกต้อง'));
      expect(getRefusalText()).toBe(screenTh.badBranch);

      // In EN
      dispose?.();
      dispose = undefined;
      host.innerHTML = '';
      setLang('en');

      await mount(refusal(400, 'invalidRequest', 'คำขอไม่ถูกต้อง'));
      expect(getRefusalText()).toBe(screenEn.badBranch);
    });

    it('field-carrying 400 invalidRequest falls through to errorText/wordedError', async () => {
      await mount(refusal(400, 'invalidRequest', 'คำขอไม่ถูกต้อง', 'ref'));
      expect(getRefusalText()).toBe(th['error.invalidRequest']);
      expect(getRefusalText()).not.toBe(screenTh.badBranch);
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // Unit tests for refusalText helper directly
  // ──────────────────────────────────────────────────────────────────────────
  describe('refusalText helper unit tests', () => {
    it('network non-ApiError returns error.network in TH and EN', () => {
      setLang('th');
      expect(refusalText(new TypeError('Failed to fetch'), tr)).toBe(th['error.network']);

      setLang('en');
      expect(refusalText(new TypeError('Failed to fetch'), tr)).toBe(en['error.network']);
    });

    it('500 with keyed non-server code returns error.serverError in TH', () => {
      setLang('th');
      const err = refusal(500, 'roleNotAllowedHere', 'database is down');
      expect(refusalText(err, tr)).toBe(th['error.serverError']);
    });

    it('unkeyed code with server message returns that message', () => {
      const err = refusal(400, 'someCustomCode', 'ข้อความปฏิเสธจากเซิร์ฟเวอร์');
      expect(refusalText(err, tr)).toBe('ข้อความปฏิเสธจากเซิร์ฟเวอร์');
    });

    it('no-field 400 invalidRequest returns documents.badBranch', () => {
      setLang('th');
      const err = refusal(400, 'invalidRequest', 'คำขอไม่ถูกต้อง');
      expect(refusalText(err, tr)).toBe(screenTh.badBranch);
    });
  });
});
