// BK-13 #6147 (docs/design/backdated-keying.md §13): the desk header shows a "คีย์ย้อนหลัง" link
// only to callers holding `exchangeBackdate`. A teller never sees it.
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../shared/i18n';
import { DeskPage } from './DeskPage';
import { createFrameStore } from './frame-store';
import { collectSteps } from './steps';

const STEP = {
  './steps/rate/step.tsx': {
    step: { index: 0, label: () => 'Rate', component: () => <div data-testid="body-0">rate body</div> },
  },
};

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

describe('BK-13 DeskPage backdated keying link', () => {
  beforeEach(() => {
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
  });

  function mount(permissions?: string[]) {
    const steps = collectSteps(STEP);
    const store = createFrameStore(steps.map((s) => s.index));
    const session = permissions !== undefined ? ({
      me: () => ({
        id: 'u-1',
        name: 'Manager',
        branch: 'MAIN',
        branchName: 'XC-B',
        roles: [],
        branches: [],
        permissions,
      }),
      signOut: async () => {},
      dispose: () => {},
    } as any) : undefined;
    dispose = render(() => <DeskPage store={store} steps={steps} session={session} />, host);
  }

  it('renders a "คีย์ย้อนหลัง" link to /desk/backdated when me.permissions includes exchangeBackdate', () => {
    mount(['exchangeBackdate']);
    const link = host.querySelector<HTMLAnchorElement>('a[href="/desk/backdated"]');
    expect(link).not.toBeNull();
    expect(link!.textContent?.trim()).toBe('คีย์ย้อนหลัง');
  });

  it('does not render the backdated link when me.permissions lacks exchangeBackdate', () => {
    mount(['BranchDayOperate', 'ShiftOperate']);
    const link = host.querySelector('a[href="/desk/backdated"]');
    expect(link).toBeNull();
  });

  it('does not render the backdated link when me.permissions is empty', () => {
    mount([]);
    const link = host.querySelector('a[href="/desk/backdated"]');
    expect(link).toBeNull();
  });

  it('does not render the backdated link when signed out / no session', () => {
    mount(undefined);
    const link = host.querySelector('a[href="/desk/backdated"]');
    expect(link).toBeNull();
  });

  it('reactively shows the link when exchangeBackdate is granted', () => {
    const [perms, setPerms] = createSignal<string[]>([]);
    const steps = collectSteps(STEP);
    const store = createFrameStore(steps.map((s) => s.index));
    const session = {
      me: () => ({
        id: 'u-1',
        name: 'User',
        branch: 'MAIN',
        branchName: 'XC-B',
        roles: [],
        branches: [],
        permissions: perms(),
      }),
      signOut: async () => {},
      dispose: () => {},
    } as any;
    dispose = render(() => <DeskPage store={store} steps={steps} session={session} />, host);
    expect(host.querySelector('a[href="/desk/backdated"]')).toBeNull();

    setPerms(['exchangeBackdate']);
    const link = host.querySelector<HTMLAnchorElement>('a[href="/desk/backdated"]');
    expect(link).not.toBeNull();
    expect(link!.textContent?.trim()).toBe('คีย์ย้อนหลัง');
  });
});
