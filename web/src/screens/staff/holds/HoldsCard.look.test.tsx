import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import {
  makeStaffFloorView,
  makeStaffHoldRow,
  makeStaffMeView,
} from '../../../shared/testing/fixtures';
import type { StaffCtx } from '../cards';
import { HoldsCard } from './HoldsCard';

function me(permissions = ['CustomerHoldPlace', 'VerificationReview']) {
  return makeStaffMeView({
    userId: '00000000-0000-4000-8000-0000000000c1',
    name: 'Ploy',
    roles: [],
    branchId: 'MAIN',
    branchName: 'XC-B',
    permissions,
  });
}

function floor() {
  return makeStaffFloorView({
    phase: 'OPEN',
    hours: null,
    myClockIn: '2026-10-11T01:00:00Z',
    myShift: null,
    cutoff: null,
    target: null,
    safe: null,
  });
}

function makeCtx(perms?: string[]): StaffCtx {
  return {
    me: me(perms),
    floor: floor(),
    refresh: vi.fn(async () => {}),
  } as unknown as StaffCtx;
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function view(data: unknown): Response {
  return json(200, { version: 1, changed: true, data });
}

describe('HoldsCard look tests (#5135)', () => {
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
    setLang('th');
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  async function flush(): Promise<void> {
    for (let i = 0; i < 8; i++) {
      await new Promise((r) => setTimeout(r, 0));
    }
  }

  it('renders root xc-holds-card, xc-holds-header, h2.xc-holds-title, labelled inputs with placeholders, and styled buttons', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => view([])),
    );

    dispose = render(() => <HoldsCard ctx={makeCtx()} />, host);
    await flush();

    const root = host.querySelector('.xc-holds-card');
    expect(root).not.toBeNull();

    const header = host.querySelector('.xc-holds-header');
    expect(header).not.toBeNull();

    const title = header?.querySelector('h2.xc-holds-title');
    expect(title).not.toBeNull();
    expect(title?.textContent).toBe('พักบัญชีลูกค้า');

    const form = host.querySelector('[data-testid="holds-form"].xc-holds-form');
    expect(form).not.toBeNull();

    const keyInput = host.querySelector<HTMLInputElement>('[data-testid="holds-key"]');
    expect(keyInput).not.toBeNull();
    expect(keyInput?.classList.contains('xc-holds-input')).toBe(true);
    expect(keyInput?.placeholder).toBeTruthy();

    const findBtn = host.querySelector<HTMLButtonElement>('[data-testid="holds-find"]');
    expect(findBtn).not.toBeNull();
    expect(findBtn?.classList.contains('xc-holds-btn')).toBe(true);

    const reasonInput = host.querySelector<HTMLInputElement>('[data-testid="holds-reason"]');
    expect(reasonInput).not.toBeNull();
    expect(reasonInput?.classList.contains('xc-holds-input')).toBe(true);
    expect(reasonInput?.placeholder).toBeTruthy();

    const placeBtn = host.querySelector<HTMLButtonElement>('[data-testid="holds-place"]');
    expect(placeBtn).not.toBeNull();
    expect(placeBtn?.classList.contains('xc-holds-save-btn')).toBe(true);
  });

  it('renders English title when language is en', async () => {
    setLang('en');
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => view([])),
    );

    dispose = render(() => <HoldsCard ctx={makeCtx()} />, host);
    await flush();

    const title = host.querySelector('.xc-holds-title');
    expect(title?.textContent).toBe('Customer holds');

    const keyInput = host.querySelector<HTMLInputElement>('[data-testid="holds-key"]');
    expect(keyInput?.placeholder).toBe('Name, customer ID or passport');
  });

  it('words hold status: RELEASED -> ปลดพักแล้ว (never raw RELEASED), OPEN -> พักอยู่, unknown as is', async () => {
    const holdsData = [
      makeStaffHoldRow({
        id: 'h-1',
        customerId: 'CX-1',
        customerName: 'Customer 1',
        reason: 'Suspicious activity',
        status: 'RELEASED',
      }),
      makeStaffHoldRow({
        id: 'h-2',
        customerId: 'CX-2',
        customerName: 'Customer 2',
        reason: 'Pending verification',
        status: 'OPEN',
      }),
      makeStaffHoldRow({
        id: 'h-3',
        customerId: 'CX-3',
        customerName: 'Customer 3',
        reason: 'Special review',
        status: 'OTHER',
      }),
    ];

    vi.stubGlobal(
      'fetch',
      vi.fn(async () => view(holdsData)),
    );

    dispose = render(() => <HoldsCard ctx={makeCtx()} />, host);
    await flush();

    const row1 = host.querySelector('[data-hold-id="h-1"]');
    expect(row1).not.toBeNull();
    expect(row1?.classList.contains('xc-holds-row')).toBe(true);
    expect(row1?.textContent).toContain('ปลดพักแล้ว');
    expect(row1?.textContent).not.toContain('RELEASED');

    const row2 = host.querySelector('[data-hold-id="h-2"]');
    expect(row2).not.toBeNull();
    expect(row2?.classList.contains('xc-holds-row')).toBe(true);
    expect(row2?.textContent).toContain('พักอยู่');
    expect(row2?.textContent).not.toContain('OPEN');

    const releaseBtn = row2?.querySelector('[data-testid="holds-release"]');
    expect(releaseBtn).not.toBeNull();
    expect(releaseBtn?.classList.contains('xc-holds-btn')).toBe(true);

    const row3 = host.querySelector('[data-hold-id="h-3"]');
    expect(row3).not.toBeNull();
    expect(row3?.classList.contains('xc-holds-row')).toBe(true);
    expect(row3?.textContent).toContain('OTHER');
  });

  it('words hold status in English when lang is en', async () => {
    setLang('en');
    const holdsData = [
      makeStaffHoldRow({
        id: 'h-1',
        customerId: 'CX-1',
        customerName: 'Customer 1',
        reason: 'Suspicious activity',
        status: 'RELEASED',
      }),
      makeStaffHoldRow({
        id: 'h-2',
        customerId: 'CX-2',
        customerName: 'Customer 2',
        reason: 'Pending verification',
        status: 'OPEN',
      }),
    ];

    vi.stubGlobal(
      'fetch',
      vi.fn(async () => view(holdsData)),
    );

    dispose = render(() => <HoldsCard ctx={makeCtx()} />, host);
    await flush();

    const row1 = host.querySelector('[data-hold-id="h-1"]');
    expect(row1?.textContent).toContain('Released');
    expect(row1?.textContent).not.toContain('RELEASED');

    const row2 = host.querySelector('[data-hold-id="h-2"]');
    expect(row2?.textContent).toContain('On hold');
    expect(row2?.textContent).not.toContain('OPEN');
  });
});
