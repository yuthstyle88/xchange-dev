// BT-F3 (kiosk-shared-phone §7.3 T-D5, T-D10; §7.7; §7.8 T1, T6–T8; §7.9):
// Behaviour tests for the branch tablets panel on the admin branches screen.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../shared/api/client';
import { setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import { sharedSession } from '../../../shared/session/session';
import {
  makeAdminBranchItem,
  makeAdminBranchesData,
} from '../../../shared/testing/fixtures/admin_branches';
import { makeAdminBranchProfileData } from '../../../shared/testing/fixtures/admin_branches_profile';
import { makeMe } from '../../../shared/testing/fixtures/common';
import { QrCode } from '../../../shared/ui/QrCode';
import { BranchesScreen } from './BranchesScreen';
import { BranchTablets, type TabletRow } from './BranchTablets';
import { th } from './i18n';

const B01 = 'XC-B01';
const B02 = 'XC-B02';

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function renderQrSvg(value: string, label: string): string {
  const host = document.createElement('div');
  const dispose = render(() => <QrCode value={value} label={label} />, host);
  const svg = host.querySelector('[role="img"]')?.innerHTML ?? '';
  dispose();
  return svg;
}

describe('BT-F3 BranchTablets (§7)', () => {
  let disposers: Array<() => void> = [];
  let calls: Array<{ url: string; method: string; body: unknown; headers: Headers }>;
  let tablets: TabletRow[];
  let profileCodeByBranch: Record<string, string | null>;
  let refuseAction: 'add' | 'newCode' | 'revoke' | null;

  beforeEach(() => {
    setLang('th');
    calls = [];
    refuseAction = null;
    profileCodeByBranch = {
      [B01]: 'SILOM',
      [B02]: 'SIAM',
    };
    tablets = [
      {
        tabletId: 'tab-pending',
        branchId: B01,
        name: 'Silom Tablet 1',
        state: 'PENDING',
        lastSeenAt: null,
      },
      {
        tabletId: 'tab-paired',
        branchId: B01,
        name: 'Silom Tablet 2',
        state: 'PAIRED',
        lastSeenAt: '2026-10-22T02:00:00Z',
      },
      {
        tabletId: 'tab-revoked',
        branchId: B01,
        name: 'Silom Tablet 3',
        state: 'REVOKED',
        lastSeenAt: '2026-10-21T09:30:00Z',
      },
      {
        tabletId: 'tab-other-branch',
        branchId: B02,
        name: 'Siam Tablet 1',
        state: 'PAIRED',
        lastSeenAt: '2026-10-22T01:00:00Z',
      },
    ];

    sharedSession().updateMe(
      makeMe({ permissions: ['BranchesManage', 'SystemSettings'] }),
    );

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url =
          typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const method = (init?.method ?? 'GET').toUpperCase();
        const headers = new Headers(init?.headers);
        const body = init?.body ? JSON.parse(String(init.body)) : undefined;
        calls.push({ url, method, body, headers });

        const u = new URL(url, 'http://localhost');
        if (u.pathname === '/api/v3/views/admin_branches') {
          return json({
            version: 1,
            changed: true,
            data: makeAdminBranchesData({
              branches: [
                makeAdminBranchItem({ id: B01, name: 'Silom Branch' }),
                makeAdminBranchItem({ id: B02, name: 'Siam Branch' }),
              ],
            }),
          });
        }

        if (u.pathname === '/api/v3/views/admin_branches_profile') {
          const branch = u.searchParams.get('branch') ?? B01;
          return json({
            version: 1,
            changed: true,
            data: makeAdminBranchProfileData({
              branch,
              name: branch === B02 ? 'Siam Branch' : 'Silom Branch',
              code: profileCodeByBranch[branch] ?? null,
              address: '1 Silom Rd',
              timezone: 'Asia/Bangkok',
              bookingEnabled: true,
            }),
          });
        }

        if (u.pathname === '/api/branch-tablets' && method === 'POST') {
          const req = body as {
            action?: string;
            payload?: { branchId?: string; name?: string; tabletId?: string };
          };
          if (req.action && req.action === refuseAction) {
            return json(
              { error: { code: 'ownerOnly', message: 'เฉพาะเจ้าของระบบเท่านั้น' } },
              403,
            );
          }
          if (req.action === 'list') {
            return json({ tablets: [...tablets] });
          }
          if (req.action === 'add') {
            const addedId = 'tab-added';
            tablets = [
              ...tablets,
              {
                tabletId: addedId,
                branchId: req.payload?.branchId ?? B01,
                name: req.payload?.name ?? '',
                state: 'PENDING',
                lastSeenAt: null,
              },
            ];
            return json({
              tabletId: addedId,
              code: 'ABCDE-12345',
              codeExpiresAt: '2026-10-22T03:10:00Z',
            });
          }
          if (req.action === 'newCode') {
            return json({
              tabletId: req.payload?.tabletId ?? 'tab-paired',
              code: 'NEWCD-67890',
              codeExpiresAt: '2026-10-22T03:20:00Z',
            });
          }
          if (req.action === 'revoke') {
            tablets = tablets.map((t) =>
              t.tabletId === req.payload?.tabletId ? { ...t, state: 'REVOKED' } : t,
            );
            return json({ saved: true });
          }
        }

        return json({ error: { code: 'notFound', message: 'not found' } }, 404);
      }),
    );
  });

  afterEach(() => {
    while (disposers.length) disposers.pop()?.();
    document.body.innerHTML = '';
    sharedSession().updateMe(makeMe({ permissions: [] }));
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  function mountScreen(initialBranch = B01) {
    const [branch, setBranch] = createSignal(initialBranch);
    const [shown] = createSignal(true);
    const period = () => ({ from: '2026-10-01', to: '2026-10-31' });
    const container = document.createElement('div');
    document.body.appendChild(container);
    const dispose = render(
      () => <BranchesScreen branch={branch} period={period} shown={shown} />,
      container,
    );
    disposers.push(dispose);
    return { container, setBranch };
  }

  function tabletCalls() {
    return calls.filter((c) => c.url.includes('/api/branch-tablets'));
  }

  it('V1: without SystemSettings in me.permissions, does not mount the tablet panel or POST /api/branch-tablets', async () => {
    sharedSession().updateMe(makeMe({ permissions: ['BranchesManage'] }));
    const { container } = mountScreen();

    await vi.waitFor(() => {
      expect(container.querySelector('form.branches-form')).not.toBeNull();
    });

    expect(container.querySelector('[data-section="branch-tablets"]')).toBeNull();
    expect(tabletCalls()).toHaveLength(0);
  });

  it('V2: with SystemSettings and branch code SILOM, sends one POST /api/branch-tablets {action:"list"} and shows only selected branch rows with state and lastSeenAt', async () => {
    const { container } = mountScreen(B01);

    await vi.waitFor(() => {
      expect(container.querySelectorAll('.branches-tablets-row').length).toBe(3);
    });

    const listCalls = tabletCalls();
    expect(listCalls).toHaveLength(1);
    expect(listCalls[0]?.method).toBe('POST');
    expect(listCalls[0]?.body).toEqual({ action: 'list' });
    expect(listCalls[0]?.headers.get('x-xchange-request')).toBe('1');

    const panel = container.querySelector('[data-section="branch-tablets"]')!;
    expect(panel.textContent).toContain('Silom Tablet 1');
    expect(panel.textContent).toContain(th.tabletStatePending);
    expect(panel.textContent).toContain(th.tabletNeverSeen);

    expect(panel.textContent).toContain('Silom Tablet 2');
    expect(panel.textContent).toContain(th.tabletStatePaired);
    expect(panel.textContent).toContain('2026-10-22T02:00:00Z');

    expect(panel.textContent).toContain('Silom Tablet 3');
    expect(panel.textContent).toContain(th.tabletStateRevoked);
    expect(panel.textContent).toContain('2026-10-21T09:30:00Z');

    expect(panel.textContent).not.toContain('Siam Tablet 1');
  });

  it('V3: when the branch profile code is null, the tablet panel is not mounted', async () => {
    profileCodeByBranch[B01] = null;
    const { container } = mountScreen(B01);

    await vi.waitFor(() => {
      expect(container.querySelector('form.branches-form')).not.toBeNull();
    });

    expect(container.querySelector('[data-section="branch-tablets"]')).toBeNull();
    expect(tabletCalls()).toHaveLength(0);
  });

  it('T1: typing a name and pressing add posts {action:"add", payload:{branchId, name}}, shows code + QrCode + codeExpiresAt + tabletCodeOnce, and re-lists with the new PENDING row', async () => {
    const { container } = mountScreen(B01);

    await vi.waitFor(() => {
      expect(container.querySelectorAll('.branches-tablets-row').length).toBe(3);
    });

    const input = container.querySelector<HTMLInputElement>('input[name="tabletName"]')!;
    input.value = 'Counter iPad';
    input.dispatchEvent(new Event('input', { bubbles: true }));

    const addForm = container.querySelector<HTMLFormElement>('form.branches-tablets-add')!;
    addForm.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));

    await vi.waitFor(() => {
      expect(container.querySelectorAll('.branches-tablets-row').length).toBe(4);
    });

    const addCalls = tabletCalls().filter(
      (c) => (c.body as { action?: string })?.action === 'add',
    );
    expect(addCalls).toHaveLength(1);
    expect(addCalls[0]?.body).toEqual({
      action: 'add',
      payload: { branchId: B01, name: 'Counter iPad' },
    });

    const codeBox = container.querySelector('[data-section="tablet-code"]')!;
    expect(codeBox).not.toBeNull();
    expect(codeBox.textContent).toContain('ABCDE-12345');
    expect(codeBox.textContent).toContain('2026-10-22T03:10:00Z');
    expect(codeBox.textContent).toContain(th.tabletCodeOnce);

    const expectedQrValue = `${window.location.origin}/app/queue?branch=SILOM#pair=ABCDE-12345`;
    expect(codeBox.textContent).toContain(expectedQrValue);
    const qrImg = codeBox.querySelector('[role="img"]');
    expect(qrImg?.getAttribute('aria-label')).toBe(th.tabletQrLabel);
    expect(qrImg?.innerHTML).toBe(renderQrSvg(expectedQrValue, th.tabletQrLabel));

    const newRow = container.querySelector('[data-tablet-id="tab-added"]');
    expect(newRow?.textContent).toContain('Counter iPad');
    expect(newRow?.textContent).toContain(th.tabletStatePending);
  });

  it('T1b: pressing add with an empty name sends no request and shows tabletNameRequired', async () => {
    const { container } = mountScreen(B01);

    await vi.waitFor(() => {
      expect(container.querySelectorAll('.branches-tablets-row').length).toBe(3);
    });
    const beforeCount = tabletCalls().length;

    const addForm = container.querySelector<HTMLFormElement>('form.branches-tablets-add')!;
    addForm.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));

    const panel = container.querySelector('[data-section="branch-tablets"]')!;
    const alert = panel.querySelector('[role="alert"]');
    expect(alert?.textContent).toBe(th.tabletNameRequired);
    expect(tabletCalls().length).toBe(beforeCount);
  });

  it('T7: pressing newCode on a PAIRED row posts {action:"newCode", payload:{tabletId}} and shows the new code and QR', async () => {
    const { container } = mountScreen(B01);

    await vi.waitFor(() => {
      expect(container.querySelector('[data-tablet-id="tab-paired"]')).not.toBeNull();
    });

    const pairedRow = container.querySelector('[data-tablet-id="tab-paired"]')!;
    const newCodeBtn = pairedRow.querySelector<HTMLButtonElement>(
      '[data-action="tablet-new-code"]',
    )!;
    newCodeBtn.click();

    await vi.waitFor(() => {
      expect(container.querySelector('[data-section="tablet-code"]')).not.toBeNull();
    });

    const newCodeCalls = tabletCalls().filter(
      (c) => (c.body as { action?: string })?.action === 'newCode',
    );
    expect(newCodeCalls).toHaveLength(1);
    expect(newCodeCalls[0]?.body).toEqual({
      action: 'newCode',
      payload: { tabletId: 'tab-paired' },
    });

    const codeBox = container.querySelector('[data-section="tablet-code"]')!;
    expect(codeBox.textContent).toContain('NEWCD-67890');
    expect(codeBox.textContent).toContain('2026-10-22T03:20:00Z');
    expect(codeBox.textContent).toContain(th.tabletCodeOnce);

    const expectedQrValue = `${window.location.origin}/app/queue?branch=SILOM#pair=NEWCD-67890`;
    const qrImg = codeBox.querySelector('[role="img"]');
    expect(qrImg?.innerHTML).toBe(renderQrSvg(expectedQrValue, th.tabletQrLabel));
  });

  it('T6: pressing revoke on a PAIRED row posts {action:"revoke", payload:{tabletId}}, shows tabletRevoked, and re-lists the row as REVOKED', async () => {
    const { container } = mountScreen(B01);

    await vi.waitFor(() => {
      expect(container.querySelector('[data-tablet-id="tab-paired"]')).not.toBeNull();
    });

    const pairedRow = container.querySelector('[data-tablet-id="tab-paired"]')!;
    const revokeBtn = pairedRow.querySelector<HTMLButtonElement>('[data-action="tablet-revoke"]')!;
    revokeBtn.click();

    await vi.waitFor(() => {
      const updatedRow = container.querySelector('[data-tablet-id="tab-paired"]');
      expect(updatedRow?.textContent).toContain(th.tabletStateRevoked);
    });

    const revokeCalls = tabletCalls().filter(
      (c) => (c.body as { action?: string })?.action === 'revoke',
    );
    expect(revokeCalls).toHaveLength(1);
    expect(revokeCalls[0]?.body).toEqual({
      action: 'revoke',
      payload: { tabletId: 'tab-paired' },
    });

    const panel = container.querySelector('[data-section="branch-tablets"]')!;
    expect(panel.querySelector('[role="status"]')?.textContent).toBe(th.tabletRevoked);
  });

  it('T8: server refusal (403 OwnerOnly) on add, newCode or revoke shows worded role="alert" and no code or QR', async () => {
    const { container } = mountScreen(B01);

    await vi.waitFor(() => {
      expect(container.querySelector('[data-tablet-id="tab-paired"]')).not.toBeNull();
    });

    const panel = container.querySelector('[data-section="branch-tablets"]')!;

    // 1. Refuse add
    refuseAction = 'add';
    const input = panel.querySelector<HTMLInputElement>('input[name="tabletName"]')!;
    input.value = 'Blocked Tablet';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    const addForm = panel.querySelector<HTMLFormElement>('form.branches-tablets-add')!;
    addForm.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));

    await vi.waitFor(() => {
      expect(panel.querySelector('[role="alert"]')?.textContent).toBe(sharedTh['error.ownerOnly']);
    });
    expect(panel.querySelector('[data-section="tablet-code"]')).toBeNull();
    expect(panel.querySelector('[role="img"]')).toBeNull();

    // 2. Refuse newCode
    refuseAction = 'newCode';
    const pairedRow = panel.querySelector('[data-tablet-id="tab-paired"]')!;
    pairedRow.querySelector<HTMLButtonElement>('[data-action="tablet-new-code"]')!.click();

    await vi.waitFor(() => {
      expect(panel.querySelector('[role="alert"]')?.textContent).toBe(sharedTh['error.ownerOnly']);
    });
    expect(panel.querySelector('[data-section="tablet-code"]')).toBeNull();

    // 3. Refuse revoke
    refuseAction = 'revoke';
    pairedRow.querySelector<HTMLButtonElement>('[data-action="tablet-revoke"]')!.click();

    await vi.waitFor(() => {
      expect(panel.querySelector('[role="alert"]')?.textContent).toBe(sharedTh['error.ownerOnly']);
    });
    expect(panel.querySelector('[data-section="tablet-code"]')).toBeNull();
  });

  it('once: when a code is on screen, switching branch or remounting clears the code', async () => {
    const { container } = mountScreen(B01);

    await vi.waitFor(() => {
      expect(container.querySelector('[data-tablet-id="tab-paired"]')).not.toBeNull();
    });

    container
      .querySelector('[data-tablet-id="tab-paired"] [data-action="tablet-new-code"]')!
      .dispatchEvent(new MouseEvent('click', { bubbles: true }));

    await vi.waitFor(() => {
      expect(container.querySelector('[data-section="tablet-code"]')).not.toBeNull();
    });

    // Switch branch via the branch select
    const select = container.querySelector<HTMLSelectElement>('#branch-select')!;
    select.value = B02;
    select.dispatchEvent(new Event('change', { bubbles: true }));

    await vi.waitFor(() => {
      expect(container.textContent).toContain('Siam Tablet 1');
    });
    expect(container.querySelector('[data-section="tablet-code"]')).toBeNull();

    // Remount also starts with no code shown
    const remounted = mountScreen(B01);
    await vi.waitFor(() => {
      expect(remounted.container.querySelector('[data-tablet-id="tab-paired"]')).not.toBeNull();
    });
    expect(remounted.container.querySelector('[data-section="tablet-code"]')).toBeNull();
  });

  it('once: the panel itself drops the code when its branch changes without a remount', async () => {
    const [branchId, setBranchId] = createSignal(B01);
    const host = document.createElement('div');
    document.body.appendChild(host);
    disposers.push(render(() => <BranchTablets branchId={branchId()} branchCode="SILOM" />, host));

    await vi.waitFor(() => {
      expect(host.querySelector('[data-tablet-id="tab-paired"]')).not.toBeNull();
    });
    host
      .querySelector<HTMLButtonElement>(
        '[data-tablet-id="tab-paired"] [data-action="tablet-new-code"]',
      )!
      .click();
    await vi.waitFor(() => {
      expect(host.querySelector('[data-section="tablet-code"]')).not.toBeNull();
    });

    setBranchId(B02);
    await vi.waitFor(() => {
      expect(host.textContent).toContain('Siam Tablet 1');
    });
    expect(host.querySelector('[data-section="tablet-code"]')).toBeNull();
  });

  it('gate: every write goes through ApiClient.post', async () => {
    const api = new ApiClient();
    const postSpy = vi.spyOn(api, 'post');

    const host = document.createElement('div');
    document.body.appendChild(host);
    const dispose = render(
      () => <BranchTablets branchId={B01} branchCode="SILOM" api={api} />,
      host,
    );
    disposers.push(dispose);

    await vi.waitFor(() => {
      expect(host.querySelector('[data-tablet-id="tab-paired"]')).not.toBeNull();
    });

    const input = host.querySelector<HTMLInputElement>('input[name="tabletName"]')!;
    input.value = 'Gate Tablet';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    host
      .querySelector<HTMLFormElement>('form.branches-tablets-add')!
      .dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));

    await vi.waitFor(() => {
      expect(host.querySelector('[data-tablet-id="tab-added"]')).not.toBeNull();
    });

    host
      .querySelector<HTMLButtonElement>(
        '[data-tablet-id="tab-paired"] [data-action="tablet-new-code"]',
      )!
      .click();
    await vi.waitFor(() => {
      expect(host.textContent).toContain('NEWCD-67890');
    });

    host
      .querySelector<HTMLButtonElement>(
        '[data-tablet-id="tab-paired"] [data-action="tablet-revoke"]',
      )!
      .click();
    await vi.waitFor(() => {
      expect(host.querySelector('[role="status"]')?.textContent).toBe(th.tabletRevoked);
    });

    expect(postSpy.mock.calls.map((c) => [c[0], (c[1] as { action: string }).action])).toEqual([
      ['/api/branch-tablets', 'list'],
      ['/api/branch-tablets', 'add'],
      ['/api/branch-tablets', 'list'],
      ['/api/branch-tablets', 'newCode'],
      ['/api/branch-tablets', 'revoke'],
      ['/api/branch-tablets', 'list'],
    ]);
  });
});
