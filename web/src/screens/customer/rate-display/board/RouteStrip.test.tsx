import { afterEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { BOARD_TEXT, bilingual } from './board-texts';
import type { BoardStatusView } from './board-status';
import { RouteStrip } from './RouteStrip';

const SYNCED_STATUS: BoardStatusView = {
  status: 'synced',
  ratesVisible: true,
  emptyMessage: null,
};

const DELAYED_STATUS: BoardStatusView = {
  status: 'delayed',
  ratesVisible: true,
  emptyMessage: null,
};

const OFFLINE_STATUS: BoardStatusView = {
  status: 'offline',
  ratesVisible: false,
  emptyMessage: BOARD_TEXT.askCounter,
};

describe('RouteStrip (RB-6, #3568)', () => {
  const disposers: Array<() => void> = [];

  afterEach(() => {
    for (const d of disposers) d();
    disposers.length = 0;
    document.body.innerHTML = '';
  });

  function mount(props: {
    count?: number;
    branch?: string;
    branchName?: string;
    status?: BoardStatusView;
    stale?: boolean;
  } = {}) {
    const container = document.createElement('div');
    document.body.appendChild(container);
    const dispose = render(
      () => (
        <RouteStrip
          count={props.count ?? 10}
          branch={props.branch ?? 'XC-B01'}
          branchName={props.branchName ?? 'สาขาสยาม'}
          status={props.status ?? SYNCED_STATUS}
          stale={props.stale ?? false}
        />
      ),
      container,
    );
    disposers.push(dispose);
    return container;
  }

  // Row 1: status synced / delayed / offline: the STATUS block shows that state's th and en
  it('shows th and en text for status synced, delayed, and offline', () => {
    const cSynced = mount({ status: SYNCED_STATUS });
    const statusSynced = cSynced.querySelector('[role="status"]');
    expect(statusSynced).not.toBeNull();
    expect(statusSynced?.textContent).toBe(BOARD_TEXT.synced.th);
    expect(cSynced.textContent).toContain(BOARD_TEXT.synced.en);

    const cDelayed = mount({ status: DELAYED_STATUS });
    const statusDelayed = cDelayed.querySelector('[role="status"]');
    expect(statusDelayed).not.toBeNull();
    expect(statusDelayed?.textContent).toBe(BOARD_TEXT.delayed.th);
    expect(cDelayed.textContent).toContain(BOARD_TEXT.delayed.en);

    const cOffline = mount({ status: OFFLINE_STATUS });
    const statusOffline = cOffline.querySelector('[role="status"]');
    expect(statusOffline).not.toBeNull();
    expect(statusOffline?.textContent).toBe(BOARD_TEXT.offline.th);
    expect(cOffline.textContent).toContain(BOARD_TEXT.offline.en);
  });

  // Row 2: count 37: 37 in the TO block
  it('renders count 37 in the TO block', () => {
    const container = mount({ count: 37 });
    const toBlock = container.querySelector('.xc-rb-route-to');
    expect(toBlock).not.toBeNull();
    expect(toBlock?.textContent).toContain('37');
    expect(toBlock?.textContent).toContain('currencies');
  });

  // Row 3: branch XC-B01, branchName สาขาสยาม: both shown
  it('shows both branch code XC-B01 and branchName สาขาสยาม', () => {
    const container = mount({ branch: 'XC-B01', branchName: 'สาขาสยาม' });
    expect(container.textContent).toContain('XC-B01');
    expect(container.textContent).toContain('สาขาสยาม');
  });

  // Row 4: stale: true / false: the stale line / none
  it('renders stale reference line only when stale is true', () => {
    const staleText = bilingual(BOARD_TEXT.staleReference);

    const cFalse = mount({ stale: false });
    expect(cFalse.textContent).not.toContain(staleText);

    const cTrue = mount({ stale: true });
    expect(cTrue.textContent).toContain(staleText);
  });

  // Row 5: any text: never "live", "real time", "เรียลไทม์", "实时"
  it('never contains live, real time, เรียลไทม์, or 实时 across all states', () => {
    const forbidden = ['live', 'real time', 'เรียลไทม์', '实时'];

    for (const status of [SYNCED_STATUS, DELAYED_STATUS, OFFLINE_STATUS]) {
      for (const stale of [false, true]) {
        const container = mount({ status, stale });
        expect(container.querySelector('[role="status"]')).not.toBeNull();
        const text = (container.textContent ?? '').toLowerCase();
        for (const term of forbidden) {
          expect(text).not.toContain(term.toLowerCase());
        }
      }
    }
  });
});
