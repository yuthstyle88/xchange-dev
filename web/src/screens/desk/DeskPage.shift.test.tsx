// #5641 (DESK-SHIFT T4, docs/design/desk-shift-open.md §3 T4 row D5): DeskPage's `shift` prop hides the exchange
// steps until the shift is open. Written before the implementation by the ticket author claude/studio-claude-15.
// Do not edit, delete or weaken; add tests in a file of your own.
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

describe('#5641 DeskPage shift prop', () => {
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

  function mount(shift?: { open: () => boolean; Panel: () => ReturnType<typeof DeskPage> }) {
    const steps = collectSteps(STEP);
    const store = createFrameStore(steps.map((s) => s.index));
    dispose = render(() => <DeskPage store={store} steps={steps} aside={<p data-testid="aside">sum</p>} shift={shift} />, host);
  }

  it('D5a without the prop: today\'s page (step bar, step body, aside)', () => {
    mount();
    expect(host.querySelector('ol.steps')).not.toBeNull();
    expect(host.querySelector('[data-testid="body-0"]')).not.toBeNull();
    expect(host.querySelector('.desk-aside [data-testid="aside"]')).not.toBeNull();
  });

  it('D5b not open: no step bar, no step body, no aside; the Panel takes the step\'s place', () => {
    mount({ open: () => false, Panel: () => <p data-testid="shift-panel">panel</p> });
    expect(host.querySelector('ol.steps')).toBeNull();
    expect(host.querySelector('[data-testid="body-0"]')).toBeNull();
    expect(host.querySelector('.desk-aside')).toBeNull();
    expect(host.querySelector('#panel [data-testid="shift-panel"]')).not.toBeNull();
  });

  it('D5c becoming open shows the steps and drops the Panel, without a remount of the page', () => {
    const [open, setOpen] = createSignal(false);
    mount({ open, Panel: () => <p data-testid="shift-panel">panel</p> });
    const page = host.querySelector('.desk-page');
    setOpen(true);
    expect(host.querySelector('.desk-page')).toBe(page);
    expect(host.querySelector('ol.steps')).not.toBeNull();
    expect(host.querySelector('[data-testid="body-0"]')).not.toBeNull();
    expect(host.querySelector('[data-testid="shift-panel"]')).toBeNull();
  });
});
