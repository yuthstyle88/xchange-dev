// OL-6 wave B desk-shift-gate (audit #564 issuecomment-6702970101, shots 002/003/004). Written before the fix by the
// ticket author claude/studio-claude-17 for the sub-controller claude/1cc51615. Do not edit, delete or weaken.
//
// The gate already draws inside DeskPage's shared `section.panel.card` (L7, DeskPage.tsx), so SG-5 and SG-6 guard that
// there is exactly one card. What is missing: the owner's no-shift line tells them to go to the admin page and gives no
// way there (SG-1..SG-3), and the only action of a gate step is not the primary look (SG-7).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { makeMe, makeStaffFloorView, makeStaffMeView } from '../../../shared/testing/fixtures';
import { DeskPage } from '../DeskPage';
import { createFrameStore } from '../frame-store';
import { th } from '../i18n';
import { collectSteps } from '../steps';
import { createDeskShiftGate } from './DeskShiftGate';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

const OWNER = makeMe({ id: 'u-o', name: 'Owner', branch: 'MAIN', permissions: ['ReversalRequest', 'OverviewRead'] });
const TELLER = makeMe({ id: 'u-1', name: 'Ploy', branch: 'MAIN', permissions: ['ShiftOperate', 'AttendanceClock'] });

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

function stub() {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) => {
      const path = String(input).split('?')[0];
      const view = (data: unknown) => json(200, { version: 1, changed: true, data });
      if (path === '/api/v3/views/staff_me') {
        return view(makeStaffMeView({ userId: 'u-1', name: 'Ploy', roles: [], branchId: 'MAIN', branchName: 'XC-B', permissions: ['ShiftOperate', 'AttendanceClock'] }));
      }
      if (path === '/api/v3/views/staff_floor') {
        return view(makeStaffFloorView({ phase: 'OPEN', hours: null, myClockIn: null, myShift: null, cutoff: null, target: null, safe: null }));
      }
      if (path === '/api/v3/views/cash_counts') return view([]);
      return json(404, { error: { code: 'notFound', message: String(path) } });
    }),
  );
}

async function settle() {
  for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function mountGate(me: typeof OWNER, branch: string) {
  const [m] = createSignal(me);
  dispose = render(() => createDeskShiftGate({ me: m, branch: () => branch }).Panel(), host);
}

async function readHere(file: string): Promise<string> {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  return fs.readFileSync(here + file, 'utf8');
}

function decls(css: string, selector: string): Map<string, string> | undefined {
  const re = /([^{}]+)\{([^{}]*)\}/g;
  const stripped = css.replace(/\/\*[\s\S]*?\*\//g, '');
  let m: RegExpExecArray | null;
  while ((m = re.exec(stripped)) !== null) {
    if (m[1]!.trim().replace(/\s+/g, ' ') !== selector) continue;
    const out = new Map<string, string>();
    for (const d of m[2]!.split(';')) {
      const i = d.indexOf(':');
      if (i > 0) out.set(d.slice(0, i).trim().toLowerCase(), d.slice(i + 1).trim());
    }
    return out;
  }
  return undefined;
}

const link = () => host.querySelector<HTMLAnchorElement>('a.desk-shift-admin-link');

beforeEach(() => {
  setLang('th');
  stub();
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.unstubAllGlobals();
});

describe('OL-6 B the desk shift gate', () => {
  it("SG-1 no shift access: the line itself is the link to that branch's admin page", async () => {
    mountGate(OWNER, 'MAIN');
    await settle();
    expect(link(), 'a.desk-shift-admin-link').not.toBeNull();
    expect(link()!.getAttribute('href')).toBe('/?branch=MAIN');
    expect(link()!.textContent).toBe(th['shift.noShift']);
    expect(host.textContent!.split(th['shift.noShift']).length - 1, 'the line shows once').toBe(1);
  });

  it('SG-2 a branch id with characters a URL cannot carry is encoded', async () => {
    mountGate(OWNER, 'A B/ก');
    await settle();
    expect(link()!.getAttribute('href')).toBe(`/?branch=${encodeURIComponent('A B/ก')}`);
  });

  it('SG-3 no branch: the link goes to the admin root, with no empty parameter', async () => {
    mountGate(OWNER, '');
    await settle();
    expect(link()!.getAttribute('href')).toBe('/');
  });

  it('SG-4 a teller who must clock in gets the clock button and no admin link', async () => {
    mountGate(TELLER, 'MAIN');
    await settle();
    expect(host.querySelector('.desk-shift-gate')!.getAttribute('data-desk-shift')).toBe('clockIn');
    expect(host.querySelector('.desk-actions button')).not.toBeNull();
    expect(link()).toBeNull();
  });

  it('SG-5 the gate draws no card of its own', async () => {
    mountGate(OWNER, 'MAIN');
    await settle();
    expect(host.querySelector('.card')).toBeNull();
  });

  it("SG-6 on the page the gate sits in DeskPage's one shared card", async () => {
    const steps = collectSteps({
      './steps/rate/step.tsx': { step: { index: 0, label: () => 'Rate', component: () => <div>rate</div> } },
    });
    const store = createFrameStore(steps.map((s) => s.index));
    const [m] = createSignal(OWNER);
    dispose = render(() => <DeskPage store={store} steps={steps} shift={createDeskShiftGate({ me: m, branch: () => 'MAIN' })} />, host);
    await settle();
    expect(host.querySelector('section.panel.card .desk-shift-gate')).not.toBeNull();
    expect(host.querySelectorAll('.card .card')).toHaveLength(0);
  });

  it('SG-7 the gate loads shift-gate.css; its only action is the primary look unless disabled', async () => {
    expect(await readHere('DeskShiftGate.tsx')).toMatch(/import\s+['"]\.\/shift-gate\.css['"]/);
    const d = decls(await readHere('shift-gate.css'), '.desk-shift-gate .desk-actions button:not(:disabled)');
    expect(d?.get('background')).toBe('var(--xc-brand)');
    expect(d?.get('border-color')).toBe('var(--xc-brand)');
    expect(d?.get('color')).toBe('var(--xc-on-brand)');
  });

  it('SG-8 the link is readable as a link and a 44px target', async () => {
    const d = decls(await readHere('shift-gate.css'), '.desk-shift-admin-link');
    expect(d?.get('display')).toBe('inline-flex');
    expect(d?.get('align-items')).toBe('center');
    expect(d?.get('min-height')).toBe('44px');
    expect(d?.get('color')).toBe('var(--xc-brand)');
    expect(d?.get('font-weight')).toBe('600');
  });
});
