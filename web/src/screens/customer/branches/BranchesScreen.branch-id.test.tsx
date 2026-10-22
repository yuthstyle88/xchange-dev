// CB-Branches (108-Plaza/xchange#4832): branch cards and links key by branch.id, while display and search keep branch.code.
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Fetch } from '../../../shared/api/client';
import type { PublicBranchesData } from '../../../shared/api/public/public_branches';
import {
  makeBookingSlotsData,
  makeBranchServiceStatusData,
  makeMyQueueTicketData,
} from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { BranchesScreen } from './BranchesScreen';

const BRANCHES: PublicBranchesData = {
  branches: [
    {
      id: 'br-1',
      code: 'MAIN',
      name: 'Siam',
      address: null,
      openingHours: null,
      timezone: 'Asia/Bangkok',
      bookingEnabled: true,
      queueOpen: true,
    },
  ],
};

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function server() {
  const urls: URL[] = [];
  const writes: { path: string; body: unknown }[] = [];
  const fetchFn: Fetch = vi.fn(async (input: string, init?: RequestInit) => {
    const u = new URL(input, 'http://test.local');
    urls.push(u);
    const method = (init?.method ?? 'GET').toUpperCase();
    if (u.pathname === '/api/public/orgs/XC/branches') {
      return json(200, BRANCHES);
    }
    if (u.pathname === '/api/v3/views/branch_service_status') {
      return json(200, {
        version: 1,
        changed: true,
        data: makeBranchServiceStatusData({ branch: 'br-1' }),
      });
    }
    if (u.pathname === '/api/v3/views/booking_slots') {
      return json(200, {
        version: 1,
        changed: true,
        data: makeBookingSlotsData({ slots: [] }),
      });
    }
    if (method === 'POST' && u.pathname === '/api/app/queue/tickets') {
      writes.push({ path: u.pathname, body: JSON.parse(String(init?.body ?? '{}')) });
      return json(200, makeMyQueueTicketData({ ticketId: 't-1', branch: 'br-1', displayNumber: 'A001' }));
    }
    return json(404, {});
  });
  return { fetchFn, urls, writes };
}

async function flush(): Promise<void> {
  for (let i = 0; i < 8; i += 1) await new Promise((r) => setTimeout(r, 0));
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function mount(fetchFn: Fetch): void {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <BranchesScreen fetch={fetchFn} org="XC" />, host);
}

beforeEach(() => {
  setAudience('customer');
  setLang('en');
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
});

describe('BranchesScreen branch id vs code (CB-Branches)', () => {
  it('R1: the card shows MAIN in .xc-branch-code and Siam as its name', async () => {
    const s = server();
    mount(s.fetchFn);
    await flush();
    const card = host.querySelector('.xc-branch-card')!;
    expect(card.querySelector('.xc-branch-code')?.textContent).toBe('MAIN');
    expect(card.querySelector('.xc-branch-name')?.textContent).toBe('Siam');
  });

  it('R2: the rates link is /rates?branch=br-1; the book link is /app/book?branch=br-1', async () => {
    const s = server();
    mount(s.fetchFn);
    await flush();
    expect(host.querySelector('.xc-branch-rates-link')?.getAttribute('href')).toBe('/rates?branch=br-1');
    expect(host.querySelector('.xc-branch-book-link')?.getAttribute('href')).toBe('/app/book?branch=br-1');
  });

  it('R3: the branch_service_status and booking_slots view requests carry branch=br-1', async () => {
    const s = server();
    mount(s.fetchFn);
    await flush();
    const statusUrl = s.urls.find((u) => u.pathname === '/api/v3/views/branch_service_status');
    const slotsUrl = s.urls.find((u) => u.pathname === '/api/v3/views/booking_slots');
    expect(statusUrl?.searchParams.get('branch')).toBe('br-1');
    expect(slotsUrl?.searchParams.get('branch')).toBe('br-1');
  });

  it('R4: taking a number sends POST /api/app/queue/tickets { branch: "br-1" } and links to /app/journey?branch=br-1&ticketId=…', async () => {
    const s = server();
    mount(s.fetchFn);
    await flush();
    const btn = host.querySelector<HTMLButtonElement>('button[data-action="take-number"]')!;
    btn.click();
    await flush();
    expect(s.writes).toEqual([{ path: '/api/app/queue/tickets', body: { branch: 'br-1' } }]);
    const link = host.querySelector<HTMLAnchorElement>('a[data-testid="take-number-link"]');
    expect(link?.getAttribute('href')).toBe('/app/journey?branch=br-1&ticketId=t-1');
  });

  it('R5: searching by MAIN still lists the card (search by code stays)', async () => {
    const s = server();
    mount(s.fetchFn);
    await flush();
    const input = host.querySelector<HTMLInputElement>('.xc-branches-search-input')!;
    input.value = 'MAIN';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    await flush();
    expect(host.querySelectorAll('.xc-branch-card')).toHaveLength(1);
  });
});
