import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { en } from '../../../shared/i18n/en';
import { th } from '../../../shared/i18n/th';
import { Rates } from './Rates';
import { toast, Toast } from '../parts/toast';

type Reply = { status: number; body?: unknown; reject?: Error };

function refusal(status: number, code: string, message: string): Reply {
  return { status, body: { error: { code, message } } };
}

function text(el: Element | null | undefined): string {
  return el?.textContent?.replace(/\s+/g, ' ').trim() ?? '';
}

async function settle(): Promise<void> {
  for (let round = 0; round < 3; round++) {
    for (let i = 0; i < 5; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
}

describe('WE-21: Rates screen worded error tests (web-worded-error.md)', () => {
  let root: HTMLDivElement | undefined;
  let dispose: (() => void) | undefined;
  let replies: Record<string, () => Reply>;

  beforeEach(() => {
    setLang('th');
    toast('');
    replies = {};

    vi.stubGlobal('fetch', vi.fn(async (input: string, init?: RequestInit) => {
      const url = new URL(String(input), 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      const key = `${method} ${url.pathname}`;

      if (replies[key]) {
        const rep = replies[key]!();
        if (rep.reject) throw rep.reject;
        return new Response(JSON.stringify(rep.body ?? {}), {
          status: rep.status,
          headers: { 'content-type': 'application/json' },
        });
      }

      // Default safe view responses
      const viewWrap = (data: unknown) =>
        new Response(JSON.stringify({ version: 1, changed: true, data }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });

      if (url.pathname === '/api/v3/views/published_rates') {
        return viewWrap({ lines: [{ currency: 'USD', group: 'ALL', unitSize: '1', buyRate: '33.00', sellRate: '34.00', lineId: 'l1' }] });
      }
      if (url.pathname === '/api/v3/views/rate_drafts') {
        return viewWrap({ drafts: [{ id: 'd1', branchId: 'MAIN', status: 'DRAFT', version: 1, createdBy: 'u1', createdAt: '2026-10-10', updatedAt: '2026-10-10', evaluation: null }] });
      }
      if (url.pathname === '/api/v3/views/rate_draft') {
        return viewWrap({ draft: { id: 'd1', branchId: 'MAIN', status: 'DRAFT', version: 1, createdBy: 'u1', createdAt: '2026-10-10', updatedAt: '2026-10-10', evaluation: null }, lines: [] });
      }
      if (url.pathname === '/api/v3/views/rate_policy') {
        return viewWrap({ policy: { id: 'pol-1', branchId: 'MAIN', version: 1, buyAdjustPct: '0.0000', sellAdjustPct: '0.0000', guards: {}, maxReferenceAgeHours: 24, onStale: 'STOP', staleLimitThb: null, validFrom: '2026-10-08T00:00:00+00:00', validTo: null, approvedBy: 'owner-1', reason: null, valueTiersOn: false, valueTiers: [], sharpMoveAlertPct: null } });
      }
      if (url.pathname === '/api/v3/views/rates_reference') {
        return viewWrap({ rows: [], fetchedAt: '2026-10-10T01:30:00.000Z', error: null });
      }
      return new Response(JSON.stringify({}), { status: 200, headers: { 'content-type': 'application/json' } });
    }));
  });

  afterEach(async () => {
    await settle();
    dispose?.();
    dispose = undefined;
    root?.remove();
    root = undefined;
    vi.unstubAllGlobals();
    setLang('th');
  });

  function mountRates(branchSignal?: () => string): HTMLDivElement {
    root = document.createElement('div');
    document.body.appendChild(root);
    const [shown] = createSignal(true);
    const branch = branchSignal ?? (() => 'MAIN');
    dispose = render(
      () => (
        <>
          <Toast />
          <Rates branch={branch} period={() => ({ from: '2026-10-01', to: '2026-10-31' })} shown={shown} />
        </>
      ),
      root,
    );
    return root;
  }

  function getToastText(): string {
    const tEl = document.querySelector('.toast');
    if (!tEl || tEl.hasAttribute('hidden')) return '';
    return text(tEl);
  }

  // ── Read catches ─────────────────────────────────────────────────────────

  describe('Catch 1: readPublished', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      replies['GET /api/v3/views/published_rates'] = () => ({
        status: 0,
        reject: new TypeError('Failed to fetch'),
      });
      const el = mountRates();
      await settle();

      const alerts = Array.from(el.querySelectorAll('[role="alert"]'));
      expect(alerts.some((a) => text(a).includes(th['error.network']))).toBe(true);
      expect(alerts.some((a) => text(a).includes('Failed to fetch'))).toBe(false);

      setLang('en');
      dispose?.();
      mountRates();
      await settle();
      const alertsEn = Array.from(root!.querySelectorAll('[role="alert"]'));
      expect(alertsEn.some((a) => text(a).includes(en['error.network']))).toBe(true);
      expect(alertsEn.some((a) => text(a).includes('Failed to fetch'))).toBe(false);
    });

    it('(2) 500 refusal with keyed code shows error.serverError (pins mutation to toErrorText)', async () => {
      replies['GET /api/v3/views/published_rates'] = () =>
        refusal(500, 'roleNotAllowedHere', 'pub server crashed');
      const el = mountRates();
      await settle();

      const alerts = Array.from(el.querySelectorAll('[role="alert"]'));
      expect(alerts.some((a) => text(a).includes(th['error.serverError']))).toBe(true);
      expect(alerts.some((a) => text(a).includes('สาขานี้อยู่นอกสิทธิ์ของคุณ'))).toBe(false);
    });

    it('(3) refusal with unmapped code shows server message (pins W10 (4))', async () => {
      const sentence = 'ไม่สามารถอ่านเรทปัจจุบันของสาขานี้ได้';
      replies['GET /api/v3/views/published_rates'] = () =>
        refusal(400, 'unmappedPubCode', sentence);
      const el = mountRates();
      await settle();

      const alerts = Array.from(el.querySelectorAll('[role="alert"]'));
      expect(alerts.some((a) => text(a).includes(sentence))).toBe(true);
      expect(alerts.some((a) => text(a).includes(th['error.serverError']))).toBe(false);
    });
  });

  describe('Catch 2: readDrafts', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      replies['GET /api/v3/views/rate_drafts'] = () => ({
        status: 0,
        reject: new TypeError('Network timeout'),
      });
      const el = mountRates();
      await settle();

      const alerts = Array.from(el.querySelectorAll('[role="alert"]'));
      expect(alerts.some((a) => text(a).includes(th['error.network']))).toBe(true);

      setLang('en');
      dispose?.();
      mountRates();
      await settle();
      const alertsEn = Array.from(root!.querySelectorAll('[role="alert"]'));
      expect(alertsEn.some((a) => text(a).includes(en['error.network']))).toBe(true);
    });

    it('(2) 500 refusal with keyed code shows error.serverError', async () => {
      replies['GET /api/v3/views/rate_drafts'] = () =>
        refusal(500, 'roleNotAllowedHere', 'drafts crashed');
      const el = mountRates();
      await settle();

      const alerts = Array.from(el.querySelectorAll('[role="alert"]'));
      expect(alerts.some((a) => text(a).includes(th['error.serverError']))).toBe(true);
      expect(alerts.some((a) => text(a).includes('สาขานี้อยู่นอกสิทธิ์ของคุณ'))).toBe(false);
    });

    it('(3) refusal with unmapped code shows server message (pins W10 (4))', async () => {
      const sentence = 'ไม่สามารถอ่านรายการร่างเรทได้';
      replies['GET /api/v3/views/rate_drafts'] = () =>
        refusal(400, 'unmappedDraftsCode', sentence);
      const el = mountRates();
      await settle();

      const alerts = Array.from(el.querySelectorAll('[role="alert"]'));
      expect(alerts.some((a) => text(a).includes(sentence))).toBe(true);
      expect(alerts.some((a) => text(a).includes(th['error.serverError']))).toBe(false);
    });
  });

  describe('Catch 3: readPolicy', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      replies['GET /api/v3/views/rate_policy'] = () => ({
        status: 0,
        reject: new TypeError('Failed to fetch policy'),
      });
      const el = mountRates();
      await settle();

      const alerts = Array.from(el.querySelectorAll('[role="alert"]'));
      expect(alerts.some((a) => text(a).includes(th['error.network']))).toBe(true);

      setLang('en');
      dispose?.();
      mountRates();
      await settle();
      const alertsEn = Array.from(root!.querySelectorAll('[role="alert"]'));
      expect(alertsEn.some((a) => text(a).includes(en['error.network']))).toBe(true);
    });

    it('(2) 500 refusal with keyed code shows error.serverError', async () => {
      replies['GET /api/v3/views/rate_policy'] = () =>
        refusal(500, 'roleNotAllowedHere', 'policy db down');
      const el = mountRates();
      await settle();

      const alerts = Array.from(el.querySelectorAll('[role="alert"]'));
      expect(alerts.some((a) => text(a).includes(th['error.serverError']))).toBe(true);
      expect(alerts.some((a) => text(a).includes('สาขานี้อยู่นอกสิทธิ์ของคุณ'))).toBe(false);
    });

    it('(3) refusal with unmapped code shows server message (pins W10 (4))', async () => {
      const sentence = 'นโยบายเรทยังไม่เปิดใช้งานสำหรับสาขานี้';
      replies['GET /api/v3/views/rate_policy'] = () =>
        refusal(400, 'unmappedPolicyCode', sentence);
      const el = mountRates();
      await settle();

      const alerts = Array.from(el.querySelectorAll('[role="alert"]'));
      expect(alerts.some((a) => text(a).includes(sentence))).toBe(true);
      expect(alerts.some((a) => text(a).includes(th['error.serverError']))).toBe(false);
    });
  });

  describe('Catch 4: fetchReference', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      replies['GET /api/v3/views/rates_reference'] = () => ({
        status: 0,
        reject: new TypeError('Failed to fetch reference rates'),
      });
      const el = mountRates();
      await settle();

      const botErr = el.querySelector('.bot-error');
      expect(botErr).not.toBeNull();
      expect(text(botErr)).toBe(th['error.network']);

      setLang('en');
      dispose?.();
      mountRates();
      await settle();
      const botErrEn = root!.querySelector('.bot-error');
      expect(text(botErrEn)).toBe(en['error.network']);
    });

    it('(2) 500 refusal with keyed code shows error.serverError', async () => {
      replies['GET /api/v3/views/rates_reference'] = () =>
        refusal(500, 'roleNotAllowedHere', 'ref crashed');
      const el = mountRates();
      await settle();

      const botErr = el.querySelector('.bot-error');
      expect(botErr).not.toBeNull();
      expect(text(botErr)).toBe(th['error.serverError']);
      expect(text(botErr)).not.toBe('สาขานี้อยู่นอกสิทธิ์ของคุณ');
    });

    it('(3) refusal with unmapped code shows server message (pins W10 (4))', async () => {
      const sentence = 'ไม่สามารถดึงเรทอ้างอิงของธนาคารได้';
      replies['GET /api/v3/views/rates_reference'] = () =>
        refusal(400, 'unmappedRefCode', sentence);
      const el = mountRates();
      await settle();

      const botErr = el.querySelector('.bot-error');
      expect(botErr).not.toBeNull();
      expect(text(botErr)).toBe(sentence);
      expect(text(botErr)).not.toBe(th['error.serverError']);
    });
  });

  // ── Action / Toast catches ────────────────────────────────────────────────

  describe('Catch 5: handleNewDraft', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      replies['POST /api/rate-drafts'] = () => ({
        status: 0,
        reject: new TypeError('Failed to create draft'),
      });
      const el = mountRates();
      await settle();

      const btn = el.querySelector('.btn-new-draft') as HTMLButtonElement | null;
      btn?.click();
      await settle();
      expect(getToastText()).toBe(th['error.network']);

      setLang('en');
      btn?.click();
      await settle();
      expect(getToastText()).toBe(en['error.network']);
    });

    it('(2) 500 refusal with keyed code shows error.serverError', async () => {
      replies['POST /api/rate-drafts'] = () =>
        refusal(500, 'roleNotAllowedHere', 'db failed');
      const el = mountRates();
      await settle();

      const btn = el.querySelector('.btn-new-draft') as HTMLButtonElement | null;
      btn?.click();
      await settle();

      expect(getToastText()).toBe(th['error.serverError']);
      expect(getToastText()).not.toBe('สาขานี้อยู่นอกสิทธิ์ของคุณ');
    });

    it('(3) refusal with unmapped code shows server message (pins W10 (4))', async () => {
      const sentence = 'มีร่างเรทที่รออนุมัติอยู่แล้ว';
      replies['POST /api/rate-drafts'] = () =>
        refusal(400, 'unmappedDraftCreate', sentence);
      const el = mountRates();
      await settle();

      const btn = el.querySelector('.btn-new-draft') as HTMLButtonElement | null;
      btn?.click();
      await settle();

      expect(getToastText()).toBe(sentence);
      expect(getToastText()).not.toBe(th['error.serverError']);
    });
  });

  describe('Catch 6: handleSimulate', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      replies['POST /api/rate-drafts/d1/simulate'] = () => ({
        status: 0,
        reject: new TypeError('Failed to simulate'),
      });
      const el = mountRates();
      await settle();

      const btn = el.querySelector('.btn-simulate') as HTMLButtonElement | null;
      expect(btn).not.toBeNull();
      btn?.click();
      await settle();
      expect(getToastText()).toBe(th['error.network']);

      setLang('en');
      btn?.click();
      await settle();
      expect(getToastText()).toBe(en['error.network']);
    });

    it('(2) 500 refusal with keyed code shows error.serverError', async () => {
      replies['POST /api/rate-drafts/d1/simulate'] = () =>
        refusal(500, 'roleNotAllowedHere', 'sim crashed');
      const el = mountRates();
      await settle();

      const btn = el.querySelector('.btn-simulate') as HTMLButtonElement | null;
      btn?.click();
      await settle();

      expect(getToastText()).toBe(th['error.serverError']);
      expect(getToastText()).not.toBe('สาขานี้อยู่นอกสิทธิ์ของคุณ');
    });

    it('(3) refusal with unmapped code shows server message (pins W10 (4))', async () => {
      const sentence = 'การจำลองเรทขัดข้อง กรุณาลองใหม่';
      replies['POST /api/rate-drafts/d1/simulate'] = () =>
        refusal(400, 'unmappedSimCode', sentence);
      const el = mountRates();
      await settle();

      const btn = el.querySelector('.btn-simulate') as HTMLButtonElement | null;
      btn?.click();
      await settle();

      expect(getToastText()).toBe(sentence);
      expect(getToastText()).not.toBe(th['error.serverError']);
    });
  });

  describe('Catch 7: handleSubmit', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      replies['POST /api/rate-drafts/d1/submit'] = () => ({
        status: 0,
        reject: new TypeError('Failed to submit'),
      });
      const el = mountRates();
      await settle();

      const btn = el.querySelector('.btn-submit') as HTMLButtonElement | null;
      expect(btn).not.toBeNull();
      btn?.click();
      await settle();
      expect(getToastText()).toBe(th['error.network']);

      setLang('en');
      btn?.click();
      await settle();
      expect(getToastText()).toBe(en['error.network']);
    });

    it('(2) 500 refusal with keyed code shows error.serverError', async () => {
      replies['POST /api/rate-drafts/d1/submit'] = () =>
        refusal(500, 'roleNotAllowedHere', 'sub crashed');
      const el = mountRates();
      await settle();

      const btn = el.querySelector('.btn-submit') as HTMLButtonElement | null;
      btn?.click();
      await settle();

      expect(getToastText()).toBe(th['error.serverError']);
      expect(getToastText()).not.toBe('สาขานี้อยู่นอกสิทธิ์ของคุณ');
    });

    it('(3) refusal with unmapped code shows server message (pins W10 (4))', async () => {
      const sentence = 'ไม่สามารถส่งร่างเรทเพื่อขออนุมัติได้';
      replies['POST /api/rate-drafts/d1/submit'] = () =>
        refusal(400, 'unmappedSubCode', sentence);
      const el = mountRates();
      await settle();

      const btn = el.querySelector('.btn-submit') as HTMLButtonElement | null;
      btn?.click();
      await settle();

      expect(getToastText()).toBe(sentence);
      expect(getToastText()).not.toBe(th['error.serverError']);
    });
  });

  describe('Catch 8: handlePublish', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      replies['POST /api/rate-drafts/d1/publish'] = () => ({
        status: 0,
        reject: new TypeError('Failed to publish'),
      });
      const el = mountRates();
      await settle();

      const btn = el.querySelector('.btn-publish') as HTMLButtonElement | null;
      expect(btn).not.toBeNull();
      btn?.click();
      await settle();
      expect(getToastText()).toBe(th['error.network']);

      setLang('en');
      btn?.click();
      await settle();
      expect(getToastText()).toBe(en['error.network']);
    });

    it('(2) 500 refusal with keyed code shows error.serverError', async () => {
      replies['POST /api/rate-drafts/d1/publish'] = () =>
        refusal(500, 'roleNotAllowedHere', 'pub crashed');
      const el = mountRates();
      await settle();

      const btn = el.querySelector('.btn-publish') as HTMLButtonElement | null;
      btn?.click();
      await settle();

      expect(getToastText()).toBe(th['error.serverError']);
      expect(getToastText()).not.toBe('สาขานี้อยู่นอกสิทธิ์ของคุณ');
    });

    it('(3) refusal with unmapped code shows server message (pins W10 (4))', async () => {
      const sentence = 'ไม่สามารถประกาศใช้เรทได้เนื่องจากสถานะไม่ถูกต้อง';
      replies['POST /api/rate-drafts/d1/publish'] = () =>
        refusal(400, 'unmappedPubCmdCode', sentence);
      const el = mountRates();
      await settle();

      const btn = el.querySelector('.btn-publish') as HTMLButtonElement | null;
      btn?.click();
      await settle();

      expect(getToastText()).toBe(sentence);
      expect(getToastText()).not.toBe(th['error.serverError']);
    });
  });

  describe('Catch 9: handleAdjustAll', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      replies['POST /api/rate-policies'] = () => ({
        status: 0,
        reject: new TypeError('Failed to adjust all'),
      });
      const el = mountRates();
      await settle();

      const btn = el.querySelector('.btn-adjust-all') as HTMLButtonElement | null;
      expect(btn).not.toBeNull();
      btn?.click();
      await settle();
      expect(getToastText()).toBe(th['error.network']);

      setLang('en');
      btn?.click();
      await settle();
      expect(getToastText()).toBe(en['error.network']);
    });

    it('(2) 500 refusal with keyed code shows error.serverError', async () => {
      replies['POST /api/rate-policies'] = () =>
        refusal(500, 'roleNotAllowedHere', 'policy update failed');
      const el = mountRates();
      await settle();

      const btn = el.querySelector('.btn-adjust-all') as HTMLButtonElement | null;
      btn?.click();
      await settle();

      expect(getToastText()).toBe(th['error.serverError']);
      expect(getToastText()).not.toBe('สาขานี้อยู่นอกสิทธิ์ของคุณ');
    });

    it('(3) refusal with unmapped code shows server message (pins W10 (4))', async () => {
      const sentence = 'ไม่สามารถปรับเรททั้งหมดได้ นโยบายถูกจำกัด';
      replies['POST /api/rate-policies'] = () =>
        refusal(400, 'unmappedAdjustCode', sentence);
      const el = mountRates();
      await settle();

      const btn = el.querySelector('.btn-adjust-all') as HTMLButtonElement | null;
      btn?.click();
      await settle();

      expect(getToastText()).toBe(sentence);
      expect(getToastText()).not.toBe(th['error.serverError']);
    });
  });

  describe('Catch 10: openHistory', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      replies['GET /api/v3/views/rate_publications'] = () => ({
        status: 0,
        reject: new TypeError('Failed to fetch'),
      });
      const el = mountRates();
      await settle();

      const btn = Array.from(el.querySelectorAll('button')).find((b) =>
        text(b).includes('ประวัติการปรับเรท') || text(b).includes('History'),
      );
      btn?.click();
      await settle();

      expect(getToastText()).toBe(th['error.network']);

      setLang('en');
      btn?.click();
      await settle();
      expect(getToastText()).toBe(en['error.network']);
    });

    it('(2) 500 refusal with keyed code shows error.serverError', async () => {
      replies['GET /api/v3/views/rate_publications'] = () =>
        refusal(500, 'roleNotAllowedHere', 'history service crashed');
      const el = mountRates();
      await settle();

      const btn = Array.from(el.querySelectorAll('button')).find((b) =>
        text(b).includes('ประวัติการปรับเรท') || text(b).includes('History'),
      );
      btn?.click();
      await settle();

      expect(getToastText()).toBe(th['error.serverError']);
      expect(getToastText()).not.toBe('สาขานี้อยู่นอกสิทธิ์ของคุณ');
    });

    it('(3) refusal with unmapped code shows server message (pins W10 (4))', async () => {
      const sentence = 'ไม่สามารถดึงประวัติการปรับเรทได้';
      replies['GET /api/v3/views/rate_publications'] = () =>
        refusal(400, 'unmappedHistoryCode', sentence);
      const el = mountRates();
      await settle();

      const btn = Array.from(el.querySelectorAll('button')).find((b) =>
        text(b).includes('ประวัติการปรับเรท') || text(b).includes('History'),
      );
      btn?.click();
      await settle();

      expect(getToastText()).toBe(sentence);
    });
  });

  // ── Done criterion 3 ──────────────────────────────────────────────────────

  describe('Done criterion 3: branch switch clears stale data', () => {
    it('clears previous branch published rates, drafts and policy when switching branches and read fails', async () => {
      const [branch, setBranch] = createSignal('MAIN');
      const el = mountRates(branch);
      await settle();

      // Initially published rates, drafts and adjust-all bar are visible
      expect(text(el)).toContain('33.00');
      expect(el.querySelector('[data-published-line="USD"]')).not.toBeNull();
      expect(el.querySelector('.btn-simulate')).not.toBeNull();
      expect(el.querySelector('.btn-adjust-all')).not.toBeNull();

      // Switch branch to B02 where published, drafts and policy fail
      replies['GET /api/v3/views/published_rates'] = () => refusal(500, 'roleNotAllowedHere', 'fail pub');
      replies['GET /api/v3/views/rate_drafts'] = () => refusal(500, 'roleNotAllowedHere', 'fail drafts');
      replies['GET /api/v3/views/rate_policy'] = () => refusal(500, 'roleNotAllowedHere', 'fail policy');

      setBranch('B02');
      await settle();

      // Previous branch's published rates, draft action buttons and adjust-all bar must not be left showing
      expect(text(el)).not.toContain('33.00');
      expect(el.querySelector('[data-published-line="USD"]')).toBeNull();
      expect(el.querySelector('.btn-simulate')).toBeNull();
      expect(el.querySelector('.btn-adjust-all')).toBeNull();
    });
  });
});
