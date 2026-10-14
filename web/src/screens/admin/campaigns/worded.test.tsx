import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { CampaignDetail } from '../../../shared/api/generated/campaign';
import type { CampaignListItem } from '../../../shared/api/generated/campaigns';
import { setLang } from '../../../shared/i18n';
import { en } from '../../../shared/i18n/en';
import { th } from '../../../shared/i18n/th';
import type { AdminMenuProps } from '../menus';
import { Campaigns } from './Campaigns';

const SAMPLE_LIST_ITEM: CampaignListItem = {
  id: 'cmp-1',
  name: 'Golden October',
  versionId: 'ver-1',
  version: 1,
  status: 'ACTIVE',
  startsAt: '2026-10-01T00:00:00Z',
  endsAt: '2026-10-31T23:59:59Z',
  budgetThb: '100000.00',
  budgetUsedThb: '25000.50',
  budgetLeftThb: '74999.50',
  uses: 42,
  maxUses: 100,
  period: {
    redemptions: 15,
    priceCostThb: '12345.67',
    rewardExpenseThb: '3000.00',
  },
};

function makeDetail(status = 'ACTIVE'): CampaignDetail {
  return {
    id: 'cmp-1',
    name: 'Golden October',
    currentVersionId: 'ver-1',
    versions: [
      {
        id: 'ver-1',
        version: 1,
        status: status as any,
        startsAt: '2026-10-01T00:00:00Z',
        endsAt: '2026-10-31T23:59:59Z',
        budgetThb: '100000.00',
        maxUses: 100,
        stackGroup: 'DEFAULT',
        priority: 10,
        branches: ['MAIN'],
        rules: [
          {
            id: 'rule-1',
            ruleNo: 1,
            benefitType: 'RATE_IMPROVEMENT',
            benefitClass: 'PRICE',
            requiresCoupon: true,
            eligibility: {},
            calculation: { spreadShare: '0.20' },
            caps: {},
          },
        ],
        simulations: [
          {
            id: 'sim-1',
            costSource: 'LEDGER',
            simulatedAt: '2026-10-02T10:00:00Z',
            input: {},
            result: {},
          },
        ],
        approval: null,
      },
    ],
  };
}

describe('WE-8: screens/admin/campaigns/ failed-call wordedError (docs/design/web-worded-error.md §4.1)', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | undefined;

  let listHandler: () => Promise<Response>;
  let detailHandler: () => Promise<Response>;
  let commandHandler: (url: string, method: string, body: unknown) => Promise<Response>;

  function jsonResponse(data: unknown, status = 200): Response {
    return new Response(JSON.stringify(data), {
      status,
      headers: { 'content-type': 'application/json' },
    });
  }

  async function settle(): Promise<void> {
    for (let i = 0; i < 8; i++) {
      await new Promise((r) => setTimeout(r, 0));
    }
  }

  function mount(initialBranch = 'MAIN') {
    const [branch, setBranch] = createSignal(initialBranch);
    const [period] = createSignal({ from: '2026-10-01', to: '2026-10-14' });
    const [shown] = createSignal(true);
    const props: AdminMenuProps = { branch, period, shown };

    dispose = render(() => <Campaigns {...props} />, container);
    return { branch, setBranch };
  }

  function getErrorText(): string | null {
    const el = container.querySelector('.xc-campaigns-error span');
    return el ? (el.textContent ?? '').trim() : null;
  }

  beforeEach(() => {
    setLang('th');
    container = document.createElement('div');
    document.body.appendChild(container);

    listHandler = () =>
      Promise.resolve(jsonResponse({ version: 1, changed: true, data: { campaigns: [SAMPLE_LIST_ITEM] } }));
    detailHandler = () =>
      Promise.resolve(jsonResponse({ version: 1, changed: true, data: { campaign: makeDetail('ACTIVE') } }));
    commandHandler = () =>
      Promise.resolve(jsonResponse({ id: 'cmp-1', version: 1, status: 'ACTIVE', coupons: [] }));

    vi.stubGlobal(
      'fetch',
      vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const u = new URL(rawUrl, 'http://localhost');
        const method = (init?.method ?? 'GET').toUpperCase();

        if (u.pathname === '/api/v3/views/campaigns') {
          return listHandler();
        }
        if (u.pathname === '/api/v3/views/campaign') {
          return detailHandler();
        }
        const body = init?.body && typeof init.body === 'string' ? JSON.parse(init.body) : init?.body;
        return commandHandler(u.pathname, method, body);
      }),
    );
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    container.remove();
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  // ──────────────────────────────────────────────────────────────────────────
  // Catch 1: loadList (view campaigns)
  // ──────────────────────────────────────────────────────────────────────────
  describe('Catch 1: loadList failures', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      listHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      mount();
      await settle();

      expect(getErrorText()).toBe(th['error.network']);
      expect(getErrorText()).not.toContain('Failed to fetch');

      dispose?.();
      setLang('en');
      mount();
      await settle();

      expect(getErrorText()).toBe(en['error.network']);
      expect(getErrorText()).not.toContain('Failed to fetch');
    });

    it('(2) 500 server error shows error.serverError in TH', async () => {
      listHandler = () =>
        Promise.resolve(jsonResponse({ error: { code: 'roleNotAllowedHere', message: 'DB down' } }, 500));
      mount();
      await settle();

      expect(getErrorText()).toBe(th['error.serverError']);
      expect(getErrorText()).not.toContain('DB down');
    });

    it('(3) refusal with unshared code and server message shows that message', async () => {
      const serverMsg = 'ข้อความเฉพาะสำหรับรายการแคมเปญที่ไม่พบ';
      listHandler = () =>
        Promise.resolve(jsonResponse({ error: { code: 'customListRefusal', message: serverMsg } }, 400));
      mount();
      await settle();

      expect(getErrorText()).toBe(serverMsg);
      expect(getErrorText()).not.toBe(th['error.serverError']);
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // Catch 2: loadCampaignDetail (view campaign)
  // ──────────────────────────────────────────────────────────────────────────
  describe('Catch 2: loadCampaignDetail failures', () => {
    it('(1) network TypeError on detail read shows error.network in TH and EN', async () => {
      detailHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      mount();
      await settle();

      const row = container.querySelector<HTMLTableRowElement>('tr[data-campaign-id="cmp-1"]');
      row?.click();
      await settle();

      expect(getErrorText()).toBe(th['error.network']);

      dispose?.();
      setLang('en');
      mount();
      await settle();

      const rowEn = container.querySelector<HTMLTableRowElement>('tr[data-campaign-id="cmp-1"]');
      rowEn?.click();
      await settle();

      expect(getErrorText()).toBe(en['error.network']);
    });

    it('(2) 500 server error on detail read shows error.serverError in TH', async () => {
      detailHandler = () =>
        Promise.resolve(jsonResponse({ error: { code: 'roleNotAllowedHere', message: 'detail crash' } }, 500));
      mount();
      await settle();

      const row = container.querySelector<HTMLTableRowElement>('tr[data-campaign-id="cmp-1"]');
      row?.click();
      await settle();

      expect(getErrorText()).toBe(th['error.serverError']);
      expect(getErrorText()).not.toContain('detail crash');
    });

    it('(3) refusal on detail read with unshared code and server message shows that message', async () => {
      const serverMsg = 'ไม่สามารถอ่านรายละเอียดแคมเปญนี้ได้ในขณะนี้';
      detailHandler = () =>
        Promise.resolve(jsonResponse({ error: { code: 'unmappedDetailFailure', message: serverMsg } }, 400));
      mount();
      await settle();

      const row = container.querySelector<HTMLTableRowElement>('tr[data-campaign-id="cmp-1"]');
      row?.click();
      await settle();

      expect(getErrorText()).toBe(serverMsg);
      expect(getErrorText()).not.toBe(th['error.serverError']);
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // Catch 3: handleCreate (POST /api/campaigns)
  // ──────────────────────────────────────────────────────────────────────────
  describe('Catch 3: handleCreate failures', () => {
    async function openCreateForm() {
      mount();
      await settle();
      const btn = Array.from(container.querySelectorAll<HTMLButtonElement>('button')).find(
        (b) => b.textContent?.includes('สร้างแคมเปญ') || b.textContent?.includes('Create campaign'),
      );
      btn?.click();
      await settle();

      const nameInput = container.querySelector<HTMLInputElement>('.xc-campaign-create input[data-field="name"]');
      if (nameInput) {
        nameInput.value = 'My New Campaign';
        nameInput.dispatchEvent(new Event('input', { bubbles: true }));
      }
    }

    it('(1) network TypeError on create shows error.network in TH and EN', async () => {
      commandHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      await openCreateForm();

      const submitBtn = container.querySelector<HTMLButtonElement>('.xc-campaign-create .xc-btn-primary');
      submitBtn?.click();
      await settle();

      expect(getErrorText()).toBe(th['error.network']);

      dispose?.();
      setLang('en');
      await openCreateForm();

      const submitBtnEn = container.querySelector<HTMLButtonElement>('.xc-campaign-create .xc-btn-primary');
      submitBtnEn?.click();
      await settle();

      expect(getErrorText()).toBe(en['error.network']);
    });

    it('(2) 500 server error on create shows error.serverError in TH', async () => {
      commandHandler = () =>
        Promise.resolve(jsonResponse({ error: { code: 'roleNotAllowedHere', message: 'create 500' } }, 500));
      await openCreateForm();

      const submitBtn = container.querySelector<HTMLButtonElement>('.xc-campaign-create .xc-btn-primary');
      submitBtn?.click();
      await settle();

      expect(getErrorText()).toBe(th['error.serverError']);
    });

    it('(3) refusal on create with unshared code and server message shows that message', async () => {
      const serverMsg = 'ชื่อแคมเปญนี้ซ้ำในระบบการตลาด';
      commandHandler = () =>
        Promise.resolve(jsonResponse({ error: { code: 'campaignNameConflict', message: serverMsg } }, 400));
      await openCreateForm();

      const submitBtn = container.querySelector<HTMLButtonElement>('.xc-campaign-create .xc-btn-primary');
      submitBtn?.click();
      await settle();

      expect(getErrorText()).toBe(serverMsg);
      expect(getErrorText()).not.toBe(th['error.serverError']);
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // Catch 4: handleSave (PUT /api/campaigns/:id/draft)
  // ──────────────────────────────────────────────────────────────────────────
  describe('Catch 4: handleSave failures', () => {
    async function openDetailAndGetAction(btnText: string) {
      mount();
      await settle();
      const row = container.querySelector<HTMLTableRowElement>('tr[data-campaign-id="cmp-1"]');
      row?.click();
      await settle();
      return Array.from(container.querySelectorAll<HTMLButtonElement>('.xc-detail-actions button')).find(
        (b) => b.textContent?.includes(btnText),
      );
    }

    it('(1) network TypeError on save shows error.network in TH and EN', async () => {
      commandHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      const saveBtn = await openDetailAndGetAction('บันทึก');
      saveBtn?.click();
      await settle();

      expect(getErrorText()).toBe(th['error.network']);

      dispose?.();
      setLang('en');
      const saveBtnEn = await openDetailAndGetAction('Save');
      saveBtnEn?.click();
      await settle();

      expect(getErrorText()).toBe(en['error.network']);
    });

    it('(2) 500 server error on save shows error.serverError in TH', async () => {
      commandHandler = () =>
        Promise.resolve(jsonResponse({ error: { code: 'roleNotAllowedHere', message: 'save failed' } }, 500));
      const saveBtn = await openDetailAndGetAction('บันทึก');
      saveBtn?.click();
      await settle();

      expect(getErrorText()).toBe(th['error.serverError']);
    });

    it('(3) refusal on save with unshared code and server message shows that message', async () => {
      const serverMsg = 'งบประมาณต้องมากกว่าศูนย์';
      commandHandler = () =>
        Promise.resolve(jsonResponse({ error: { code: 'budgetMustBePositive', message: serverMsg } }, 400));
      const saveBtn = await openDetailAndGetAction('บันทึก');
      saveBtn?.click();
      await settle();

      expect(getErrorText()).toBe(serverMsg);
      expect(getErrorText()).not.toBe(th['error.serverError']);
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // Catch 5: handleSimulate (POST /api/campaigns/:id/simulate)
  // ──────────────────────────────────────────────────────────────────────────
  describe('Catch 5: handleSimulate failures', () => {
    async function openDetailForSimulate() {
      detailHandler = () =>
        Promise.resolve(jsonResponse({ version: 1, changed: true, data: { campaign: makeDetail('DRAFT') } }));
      mount();
      await settle();
      const row = container.querySelector<HTMLTableRowElement>('tr[data-campaign-id="cmp-1"]');
      row?.click();
      await settle();
      return Array.from(container.querySelectorAll<HTMLButtonElement>('.xc-detail-actions button')).find(
        (b) => b.textContent?.includes('จำลองผล') || b.textContent?.includes('Simulate'),
      );
    }

    it('(1) network TypeError on simulate shows error.network in TH and EN', async () => {
      commandHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      const btn = await openDetailForSimulate();
      btn?.click();
      await settle();

      expect(getErrorText()).toBe(th['error.network']);

      dispose?.();
      setLang('en');
      const btnEn = await openDetailForSimulate();
      btnEn?.click();
      await settle();

      expect(getErrorText()).toBe(en['error.network']);
    });

    it('(2) 500 server error on simulate shows error.serverError in TH', async () => {
      commandHandler = () =>
        Promise.resolve(jsonResponse({ error: { code: 'roleNotAllowedHere', message: 'simulate crash' } }, 500));
      const btn = await openDetailForSimulate();
      btn?.click();
      await settle();

      expect(getErrorText()).toBe(th['error.serverError']);
    });

    it('(3) refusal on simulate with unshared code and server message shows that message', async () => {
      const serverMsg = 'ไม่พบข้อมูลย้อนหลังเพียงพอสำหรับการจำลองผล';
      commandHandler = () =>
        Promise.resolve(jsonResponse({ error: { code: 'insufficientLedgerData', message: serverMsg } }, 400));
      const btn = await openDetailForSimulate();
      btn?.click();
      await settle();

      expect(getErrorText()).toBe(serverMsg);
      expect(getErrorText()).not.toBe(th['error.serverError']);
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // Catch 6: handleSubmit (POST /api/campaigns/:id/submit)
  // ──────────────────────────────────────────────────────────────────────────
  describe('Catch 6: handleSubmit failures', () => {
    async function openDetailForSubmit() {
      detailHandler = () =>
        Promise.resolve(jsonResponse({ version: 1, changed: true, data: { campaign: makeDetail('SIMULATED') } }));
      mount();
      await settle();
      const row = container.querySelector<HTMLTableRowElement>('tr[data-campaign-id="cmp-1"]');
      row?.click();
      await settle();
      return Array.from(container.querySelectorAll<HTMLButtonElement>('.xc-detail-actions button')).find(
        (b) => b.textContent?.includes('ส่งอนุมัติ') || b.textContent?.includes('Submit'),
      );
    }

    it('(1) network TypeError on submit shows error.network in TH and EN', async () => {
      commandHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      const btn = await openDetailForSubmit();
      btn?.click();
      await settle();

      expect(getErrorText()).toBe(th['error.network']);

      dispose?.();
      setLang('en');
      const btnEn = await openDetailForSubmit();
      btnEn?.click();
      await settle();

      expect(getErrorText()).toBe(en['error.network']);
    });

    it('(2) 500 server error on submit shows error.serverError in TH', async () => {
      commandHandler = () =>
        Promise.resolve(jsonResponse({ error: { code: 'roleNotAllowedHere', message: 'submit 500' } }, 500));
      const btn = await openDetailForSubmit();
      btn?.click();
      await settle();

      expect(getErrorText()).toBe(th['error.serverError']);
    });

    it('(3) refusal on submit with unshared code and server message shows that message', async () => {
      const serverMsg = 'ผู้ส่งไม่มีสิทธิ์ส่งอนุมัติแคมเปญนี้';
      commandHandler = () =>
        Promise.resolve(jsonResponse({ error: { code: 'submitRoleDenied', message: serverMsg } }, 400));
      const btn = await openDetailForSubmit();
      btn?.click();
      await settle();

      expect(getErrorText()).toBe(serverMsg);
      expect(getErrorText()).not.toBe(th['error.serverError']);
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // Catch 7: handlePause (POST /api/campaigns/:id/pause)
  // ──────────────────────────────────────────────────────────────────────────
  describe('Catch 7: handlePause failures', () => {
    async function openDetailForPause() {
      detailHandler = () =>
        Promise.resolve(jsonResponse({ version: 1, changed: true, data: { campaign: makeDetail('ACTIVE') } }));
      mount();
      await settle();
      const row = container.querySelector<HTMLTableRowElement>('tr[data-campaign-id="cmp-1"]');
      row?.click();
      await settle();
      return Array.from(container.querySelectorAll<HTMLButtonElement>('.xc-detail-actions button')).find(
        (b) => b.textContent?.includes('พักแคมเปญ') || b.textContent?.includes('Pause'),
      );
    }

    it('(1) network TypeError on pause shows error.network in TH and EN', async () => {
      commandHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      const btn = await openDetailForPause();
      btn?.click();
      await settle();

      expect(getErrorText()).toBe(th['error.network']);

      dispose?.();
      setLang('en');
      const btnEn = await openDetailForPause();
      btnEn?.click();
      await settle();

      expect(getErrorText()).toBe(en['error.network']);
    });

    it('(2) 500 server error on pause shows error.serverError in TH', async () => {
      commandHandler = () =>
        Promise.resolve(jsonResponse({ error: { code: 'roleNotAllowedHere', message: 'pause crash' } }, 500));
      const btn = await openDetailForPause();
      btn?.click();
      await settle();

      expect(getErrorText()).toBe(th['error.serverError']);
    });

    it('(3) refusal on pause with unshared code and server message shows that message', async () => {
      const serverMsg = 'ไม่สามารถพักแคมเปญที่อยู่ในช่วงโปรโมชันพิเศษได้';
      commandHandler = () =>
        Promise.resolve(jsonResponse({ error: { code: 'cannotPauseSpecialPromo', message: serverMsg } }, 400));
      const btn = await openDetailForPause();
      btn?.click();
      await settle();

      expect(getErrorText()).toBe(serverMsg);
      expect(getErrorText()).not.toBe(th['error.serverError']);
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // Catch 8: handleResume (POST /api/campaigns/:id/resume)
  // ──────────────────────────────────────────────────────────────────────────
  describe('Catch 8: handleResume failures', () => {
    async function openDetailForResume() {
      detailHandler = () =>
        Promise.resolve(jsonResponse({ version: 1, changed: true, data: { campaign: makeDetail('PAUSED') } }));
      mount();
      await settle();
      const row = container.querySelector<HTMLTableRowElement>('tr[data-campaign-id="cmp-1"]');
      row?.click();
      await settle();
      return Array.from(container.querySelectorAll<HTMLButtonElement>('.xc-detail-actions button')).find(
        (b) => b.textContent?.includes('ทำงานต่อ') || b.textContent?.includes('Resume'),
      );
    }

    it('(1) network TypeError on resume shows error.network in TH and EN', async () => {
      commandHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      const btn = await openDetailForResume();
      btn?.click();
      await settle();

      expect(getErrorText()).toBe(th['error.network']);

      dispose?.();
      setLang('en');
      const btnEn = await openDetailForResume();
      btnEn?.click();
      await settle();

      expect(getErrorText()).toBe(en['error.network']);
    });

    it('(2) 500 server error on resume shows error.serverError in TH', async () => {
      commandHandler = () =>
        Promise.resolve(jsonResponse({ error: { code: 'roleNotAllowedHere', message: 'resume crash' } }, 500));
      const btn = await openDetailForResume();
      btn?.click();
      await settle();

      expect(getErrorText()).toBe(th['error.serverError']);
    });

    it('(3) refusal on resume with unshared code and server message shows that message', async () => {
      const serverMsg = 'แคมเปญนี้หมดอายุแล้วไม่สามารถเปิดใช้งานต่อได้';
      commandHandler = () =>
        Promise.resolve(jsonResponse({ error: { code: 'resumeExpiredCampaign', message: serverMsg } }, 400));
      const btn = await openDetailForResume();
      btn?.click();
      await settle();

      expect(getErrorText()).toBe(serverMsg);
      expect(getErrorText()).not.toBe(th['error.serverError']);
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // Catch 9: handleNewVersion (POST /api/campaigns/:id/versions)
  // ──────────────────────────────────────────────────────────────────────────
  describe('Catch 9: handleNewVersion failures', () => {
    async function openDetailForNewVersion() {
      detailHandler = () =>
        Promise.resolve(jsonResponse({ version: 1, changed: true, data: { campaign: makeDetail('ACTIVE') } }));
      mount();
      await settle();
      const row = container.querySelector<HTMLTableRowElement>('tr[data-campaign-id="cmp-1"]');
      row?.click();
      await settle();
      return Array.from(container.querySelectorAll<HTMLButtonElement>('.xc-detail-actions button')).find(
        (b) => b.textContent?.includes('สร้างเวอร์ชันใหม่') || b.textContent?.includes('New version'),
      );
    }

    it('(1) network TypeError on newVersion shows error.network in TH and EN', async () => {
      commandHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      const btn = await openDetailForNewVersion();
      btn?.click();
      await settle();

      expect(getErrorText()).toBe(th['error.network']);

      dispose?.();
      setLang('en');
      const btnEn = await openDetailForNewVersion();
      btnEn?.click();
      await settle();

      expect(getErrorText()).toBe(en['error.network']);
    });

    it('(2) 500 server error on newVersion shows error.serverError in TH', async () => {
      commandHandler = () =>
        Promise.resolve(jsonResponse({ error: { code: 'roleNotAllowedHere', message: 'version crash' } }, 500));
      const btn = await openDetailForNewVersion();
      btn?.click();
      await settle();

      expect(getErrorText()).toBe(th['error.serverError']);
    });

    it('(3) refusal on newVersion with unshared code and server message shows that message', async () => {
      const serverMsg = 'มีเวอร์ชันร่างอยู่แล้ว ไม่สามารถสร้างเพิ่มได้';
      commandHandler = () =>
        Promise.resolve(jsonResponse({ error: { code: 'draftAlreadyExists', message: serverMsg } }, 400));
      const btn = await openDetailForNewVersion();
      btn?.click();
      await settle();

      expect(getErrorText()).toBe(serverMsg);
      expect(getErrorText()).not.toBe(th['error.serverError']);
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // Catch 10: handleIssueCoupons (POST /api/campaigns/:id/coupons)
  // ──────────────────────────────────────────────────────────────────────────
  describe('Catch 10: handleIssueCoupons failures', () => {
    async function openDetailForIssueCoupons() {
      detailHandler = () =>
        Promise.resolve(jsonResponse({ version: 1, changed: true, data: { campaign: makeDetail('ACTIVE') } }));
      mount();
      await settle();
      const row = container.querySelector<HTMLTableRowElement>('tr[data-campaign-id="cmp-1"]');
      row?.click();
      await settle();
      return Array.from(container.querySelectorAll<HTMLButtonElement>('button')).find(
        (b) => b.textContent?.includes('ออกคูปอง') || b.textContent?.includes('Issue coupons'),
      );
    }

    it('(1) network TypeError on issue coupons shows error.network in TH and EN', async () => {
      commandHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      const btn = await openDetailForIssueCoupons();
      btn?.click();
      await settle();

      expect(getErrorText()).toBe(th['error.network']);

      dispose?.();
      setLang('en');
      const btnEn = await openDetailForIssueCoupons();
      btnEn?.click();
      await settle();

      expect(getErrorText()).toBe(en['error.network']);
    });

    it('(2) 500 server error on issue coupons shows error.serverError in TH', async () => {
      commandHandler = () =>
        Promise.resolve(jsonResponse({ error: { code: 'roleNotAllowedHere', message: 'coupon crash' } }, 500));
      const btn = await openDetailForIssueCoupons();
      btn?.click();
      await settle();

      expect(getErrorText()).toBe(th['error.serverError']);
    });

    it('(3) refusal on issue coupons with unshared code and server message shows that message', async () => {
      const serverMsg = 'จำนวนคูปองที่ออกเกินขีดจำกัดของแคมเปญ';
      commandHandler = () =>
        Promise.resolve(jsonResponse({ error: { code: 'couponLimitExceeded', message: serverMsg } }, 400));
      const btn = await openDetailForIssueCoupons();
      btn?.click();
      await settle();

      expect(getErrorText()).toBe(serverMsg);
      expect(getErrorText()).not.toBe(th['error.serverError']);
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // §4.3: Branch switch stale data handling
  // ──────────────────────────────────────────────────────────────────────────
  describe('§4.3: Branch switch stale data handling', () => {
    it('after branch switch, a failed read does not leave the previous branch data showing', async () => {
      const { setBranch } = mount('MAIN');
      await settle();

      expect(container.textContent).toContain('Golden October');

      listHandler = () =>
        Promise.resolve(jsonResponse({ error: { code: 'roleNotAllowedHere', message: 'Branch 2 failed' } }, 500));
      setBranch('B02');
      await settle();

      expect(getErrorText()).toBe(th['error.serverError']);
      expect(container.textContent).not.toContain('Golden October');
    });

    it('after branch switch, previous branch campaigns list is cleared while new branch read is pending', async () => {
      const { setBranch } = mount('MAIN');
      await settle();

      expect(container.textContent).toContain('Golden October');

      listHandler = () => new Promise(() => {});
      setBranch('B02');
      await settle();

      expect(container.textContent).not.toContain('Golden October');
    });

    it('after branch switch, open campaign detail is cleared', async () => {
      const { setBranch } = mount('MAIN');
      await settle();

      const row = container.querySelector<HTMLTableRowElement>('tr[data-campaign-id="cmp-1"]');
      row?.click();
      await settle();

      expect(container.querySelector('.xc-campaign-detail')).not.toBeNull();

      listHandler = () => new Promise(() => {});
      setBranch('B02');
      await settle();

      expect(container.querySelector('.xc-campaign-detail')).toBeNull();
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // WE2 (a): 5xx precedence over local ERROR_KEYS (#1844)
  // ──────────────────────────────────────────────────────────────────────────
  describe('WE2 (a): 5xx precedence over local ERROR_KEYS (#1844)', () => {
    it('500 with screen-specific code outOfScope shows error.serverError in TH', async () => {
      listHandler = () =>
        Promise.resolve(jsonResponse({ error: { code: 'outOfScope', message: 'DB down' } }, 500));
      mount();
      await settle();

      expect(getErrorText()).toBe(th['error.serverError']);
    });
  });
});

