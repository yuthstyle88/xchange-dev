import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../shared/i18n';
import { th } from '../../shared/i18n/th';
import { en } from '../../shared/i18n/en';
import { StaffReport } from './StaffReport';

describe('StaffReport: wordedError on failed calls (WE-37 #1786)', () => {
  let host: HTMLDivElement | undefined;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host?.remove();
    host = undefined;
    setLang('th');
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  async function flush(): Promise<void> {
    for (let i = 0; i < 10; i++) {
      await new Promise((r) => setTimeout(r, 10));
    }
  }

  it('(1) a network TypeError shows error.network in TH and EN', async () => {
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new TypeError('Failed to fetch')));

    dispose = render(() => <StaffReport />, host!);
    await flush();

    const refusal = host!.querySelector('.staff-report-refusal p');
    expect(refusal, 'refusal element').not.toBeNull();
    expect(refusal!.textContent).toBe(th['error.network']);

    // Switch to EN and trigger read again or verify EN wording
    setLang('en');
    host!.innerHTML = '';
    dispose?.();
    dispose = render(() => <StaffReport />, host!);
    await flush();

    const refusalEn = host!.querySelector('.staff-report-refusal p');
    expect(refusalEn, 'refusal element in EN').not.toBeNull();
    expect(refusalEn!.textContent).toBe(en['error.network']);
  });

  it('(2) a 500 refusal shows error.serverError in TH', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(
        new Response(
          JSON.stringify({ error: { code: 'serverError', message: 'internal: pool timeout' } }),
          { status: 500, headers: { 'content-type': 'application/json' } }
        )
      )
    );

    dispose = render(() => <StaffReport />, host!);
    await flush();

    const refusal = host!.querySelector('.staff-report-refusal p');
    expect(refusal, 'refusal element').not.toBeNull();
    expect(refusal!.textContent).toBe(th['error.serverError']);
    expect(refusal!.textContent).not.toContain('internal: pool timeout');
  });

  it('(3) an unknown code with a server message shows that message', async () => {
    const serverSentence = 'ไม่สามารถออกรายงานสำหรับวันที่เลือกได้เนื่องจากยังไม่มีการเปิดกะ';
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(
        new Response(
          JSON.stringify({ error: { code: 'shiftNotOpenedForDay', message: serverSentence } }),
          { status: 400, headers: { 'content-type': 'application/json' } }
        )
      )
    );

    dispose = render(() => <StaffReport />, host!);
    await flush();

    const refusal = host!.querySelector('.staff-report-refusal p');
    expect(refusal, 'refusal element').not.toBeNull();
    expect(refusal!.textContent).toBe(serverSentence);
  });

  it('a roleNotAllowed refusal shows screen-specific text in TH', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(
        new Response(
          JSON.stringify({ error: { code: 'roleNotAllowed', message: 'role not allowed' } }),
          { status: 403, headers: { 'content-type': 'application/json' } }
        )
      )
    );

    dispose = render(() => <StaffReport />, host!);
    await flush();

    const refusal = host!.querySelector('.staff-report-refusal p');
    expect(refusal, 'refusal element').not.toBeNull();
    expect(refusal!.textContent).toBe('บทบาทนี้ไม่มีสิทธิ์อ่านรายงานเงิน');
  });
});
