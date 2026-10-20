// §9 `MarketSources` (FE pricing-board PB-4, #3760; docs/design/blueprint/pricing-tiers.md §6, §9, §11).
import { afterEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { makePricingSourceDto, type PricingSourceDto } from '../../../shared/testing/fixtures/pricing_board';
import { en, th } from './i18n';
import { MarketSources } from './MarketSources';
import type { MarketSourceCode } from './pricing-labels';

const SAMPLE_SOURCES = [
  makePricingSourceDto({ source: 'TT', fetchedAt: '2026-10-19T08:00:00Z', stale: false, error: null }),
  makePricingSourceDto({ source: 'SR', fetchedAt: '2026-10-18T20:00:00Z', stale: true, error: null }),
  makePricingSourceDto({ source: 'BOT', fetchedAt: null, stale: false, error: 'source unreachable' }),
];

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  setLang('th');
});

function mount(props: {
  sources?: readonly PricingSourceDto[];
  canRefresh?: boolean;
  onRefresh?: (source: MarketSourceCode) => void;
}) {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(
    () => (
      <MarketSources
        sources={props.sources ?? SAMPLE_SOURCES}
        canRefresh={props.canRefresh ?? false}
        onRefresh={props.onRefresh ?? vi.fn()}
      />
    ),
    host,
  );
  return host;
}

describe('MarketSources (§9 PB-4)', () => {
  it('row 1: a fresh source renders a chip with its name (from i18n) and age (fetchedAt via toLocaleString)', () => {
    mount({ sources: [SAMPLE_SOURCES[0]!] });
    const chip = host.querySelector('[data-source="TT"]');
    expect(chip).not.toBeNull();
    expect(chip?.textContent).toContain(th['source.TT']);
    const expectedDate = new Date('2026-10-19T08:00:00Z').toLocaleString();
    expect(chip?.textContent).toContain(expectedDate);
    expect(chip?.textContent).not.toContain(th.stale);
    expect(chip?.textContent).not.toContain(th.sourceError);
  });

  it('row 2: stale: true marks the chip with the stale text in TH and EN', () => {
    mount({ sources: [SAMPLE_SOURCES[1]!] });
    const chip = host.querySelector('[data-source="SR"]');
    expect(chip).not.toBeNull();
    expect(chip?.textContent).toContain(th['source.SR']);
    expect(chip?.textContent).toContain(th.stale);

    // In EN
    dispose?.();
    host.remove();
    setLang('en');
    mount({ sources: [SAMPLE_SOURCES[1]!] });
    const enChip = host.querySelector('[data-source="SR"]');
    expect(enChip?.textContent).toContain(en['source.SR']);
    expect(enChip?.textContent).toContain(en.stale);
  });

  it('row 3: error set shows sourceError text and the server error message', () => {
    mount({ sources: [SAMPLE_SOURCES[2]!] });
    const chip = host.querySelector('[data-source="BOT"]');
    expect(chip).not.toBeNull();
    expect(chip?.textContent).toContain(th['source.BOT']);
    expect(chip?.textContent).toContain(th.sourceError);
    expect(chip?.textContent).toContain('source unreachable');

    // In EN
    dispose?.();
    host.remove();
    setLang('en');
    mount({ sources: [SAMPLE_SOURCES[2]!] });
    const enChip = host.querySelector('[data-source="BOT"]');
    expect(enChip?.textContent).toContain(en['source.BOT']);
    expect(enChip?.textContent).toContain(en.sourceError);
    expect(enChip?.textContent).toContain('source unreachable');
  });

  it('row 4: canRefresh renders button[name=refresh-<source>] per non-BOT source and calls onRefresh; none when false', () => {
    const onRefresh = vi.fn();
    // When canRefresh is true:
    mount({ sources: SAMPLE_SOURCES, canRefresh: true, onRefresh });

    const ttBtn = host.querySelector<HTMLButtonElement>('button[name="refresh-TT"]');
    const srBtn = host.querySelector<HTMLButtonElement>('button[name="refresh-SR"]');
    const botBtn = host.querySelector<HTMLButtonElement>('button[name="refresh-BOT"]');

    expect(ttBtn).not.toBeNull();
    expect(srBtn).not.toBeNull();
    expect(botBtn).toBeNull(); // BOT is never refreshable

    ttBtn?.click();
    expect(onRefresh).toHaveBeenCalledWith('TT');

    srBtn?.click();
    expect(onRefresh).toHaveBeenCalledWith('SR');

    // When canRefresh is false:
    dispose?.();
    host.remove();
    mount({ sources: SAMPLE_SOURCES, canRefresh: false, onRefresh });
    expect(host.querySelector('button[name="refresh-TT"]')).toBeNull();
    expect(host.querySelector('button[name="refresh-SR"]')).toBeNull();
    expect(host.querySelector('button[name="refresh-BOT"]')).toBeNull();
    expect(host.querySelectorAll('button').length).toBe(0);
  });
});
