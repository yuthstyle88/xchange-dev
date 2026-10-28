// V.9 `PricingRow` mode badge and FOLLOW-blocked chip (PV-f4, #5199; docs/design/blueprint/pricing-tiers.md §15.1).
// Written before the implementation by claude/8be3a451. Do not edit, delete or weaken these tests.
//
// [required] by these tests (ticket #5199 behaviour table G1–G2):
// - every grid row: `span.pricing-row-mode[data-mode=<mode>]` with `modeLabel(mode)`;
// - `followResult?.status === 'BLOCKED'`: `span.pricing-row-follow[data-follow="BLOCKED"]` with `followStatusLabel`;
//   any other follow result (or none): no `.pricing-row-follow`.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import {
  makePricingFollowResultDto,
  type PricingRowDto,
} from '../../../shared/testing/fixtures/pricing_board';
import { BOARD } from './__fixtures__/pricing-board.fixture';
import { th } from './i18n';
import { PricingRow } from './PricingRow';

const USD = BOARD.rows.find((r) => r.currency === 'USD')!;

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  setLang('th');
});

function mount(row: PricingRowDto) {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(
    () => (
      <table>
        <tbody>
          <PricingRow row={row} selected={false} onToggle={vi.fn()} onOpen={vi.fn()} />
        </tbody>
      </table>
    ),
    host,
  );
}

describe('PricingRow mode', () => {
  it('G1 mode FOLLOW, followResult BLOCKED: .pricing-row-mode[data-mode="FOLLOW"] and .pricing-row-follow[data-follow="BLOCKED"]', () => {
    mount({
      ...USD,
      mode: 'FOLLOW',
      followResult: makePricingFollowResultDto({ currency: 'USD', status: 'BLOCKED', reasons: ['STALE_SOURCE'] }),
    });
    const badge = host.querySelector('span.pricing-row-mode[data-mode="FOLLOW"]');
    expect(badge, 'span.pricing-row-mode[data-mode="FOLLOW"] exists').not.toBeNull();
    expect(badge!.textContent).toContain(th['mode.FOLLOW']);
    const chip = host.querySelector('span.pricing-row-follow[data-follow="BLOCKED"]');
    expect(chip, 'span.pricing-row-follow[data-follow="BLOCKED"] exists').not.toBeNull();
    expect(chip!.textContent).toContain(th['followStatus.BLOCKED']);
  });

  it('G2 followResult PUBLISHED: no .pricing-row-follow', () => {
    mount({
      ...USD,
      mode: 'FOLLOW',
      followResult: makePricingFollowResultDto({ currency: 'USD', status: 'PUBLISHED' }),
    });
    // the mode badge is still there
    expect(host.querySelector('span.pricing-row-mode[data-mode="FOLLOW"]')).not.toBeNull();
    expect(host.querySelector('.pricing-row-follow')).toBeNull();
  });
});
