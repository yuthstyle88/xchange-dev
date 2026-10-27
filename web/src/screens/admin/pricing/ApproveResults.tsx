// §15.8a ApproveResults: per-currency and per-branch approve answer (V.9 PV-c, #5197).
import { For, Show, type JSX } from 'solid-js';
import { ApiError } from '../../../shared/api/client';
import type { PricingApproveOutput } from '../../../shared/api/generated/pricing_approve';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import { en, th } from './i18n';
import { approveStateLabel, branchPushStateLabel } from './pricing-labels';
import './approve-results.css';

export interface ApproveResultsProps {
  out: PricingApproveOutput;
}

const tr = screenT('admin.pricing', { th, en });

function worded(code: string): string {
  return wordedError(new ApiError({ status: 409, code, message: '' }));
}

export function ApproveResults(props: ApproveResultsProps): JSX.Element {
  return (
    <div class="approve-results">
      <strong class="approve-results-title">{tr('approveResultTitle')}</strong>
      <For each={props.out.currencies}>
        {(c) => (
          <div class="approve-results-item">
            <div class="approve-results-line">
              <span>{c.currency}</span>
              <span>{approveStateLabel(c.state)}</span>
              <Show when={c.state === 'REFUSED' && c.error}>
                {(code) => <span>{worded(code())}</span>}
              </Show>
            </div>
            <Show when={c.branches && c.branches.length > 0}>
              <div class="approve-results-branches">
                <div class="approve-results-branch-title">{tr('branchPushTitle')}</div>
                <For each={c.branches}>
                  {(b) => (
                    <div class="approve-results-branch">
                      <span>{b.branchId}</span>
                      <span>{branchPushStateLabel(b.state)}</span>
                      <Show when={b.state === 'FAILED' && b.error}>
                        {(code) => <span>{worded(code())}</span>}
                      </Show>
                    </div>
                  )}
                </For>
              </div>
            </Show>
          </div>
        )}
      </For>
    </div>
  );
}
