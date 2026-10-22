// C14: the TERMS text a visitor accepts on sign-up, from the public consent texts
// (design customer-identity-consent.md `:163`; section-b-gaps.md S6; BE contract #3409:
// `GET /api/public/orgs/{org}/consent_texts?lang=` answers `[{purpose, version, body}]`).
import { createSignal, Match, onMount, Switch, type JSX } from 'solid-js';
import { ApiClient, type Fetch } from '../../../shared/api/client';
import { readHostOrg } from '../../../shared/api/public-host';
import { lang, screenT, setAudience } from '../../../shared/i18n';
import * as table from './i18n';

setAudience('customer');

const tr = screenT('customer.sign-in', table);

/** One item of the public answer, as #3409 states it. The public routes are hand-written JSON with no ts-rs type. */
export interface PublicConsentText {
  purpose: 'TERMS' | 'MARKETING';
  version: number;
  body: string;
}

/**
 * SF-5 (docs/design/web-silent-failures.md §6, owner 2026-10-22 "A"): where the TERMS text is.
 * - `loading`: a read of the host and the texts is in flight (on mount, and again on retry);
 * - `ready`: the TERMS item in the page language was found, and its body and version are shown;
 * - `missing`: a 200 answer that is an array with no TERMS item (`[]`, or only MARKETING);
 * - `failed`: the host read failed (a refusal, a network failure, or no org), the texts read failed (non-2xx or
 *   network), or the texts answer is not an array.
 * Sign-up is allowed only in `ready`.
 */
export type TermsState = 'loading' | 'ready' | 'missing' | 'failed';

export interface TermsTextProps {
  /** The sign-up screen's fetch, so tests mock at the fetch level. */
  fetch?: typeof fetch;
  api?: Pick<ApiClient, 'publicGet'>;
  /**
   * SF-5: called with `loading` each time a read starts (on mount and on retry), then with that read's outcome
   * (`ready`, `missing` or `failed`).
   */
  onState?: (state: TermsState) => void;
}

export function TermsText(props: TermsTextProps = {}): JSX.Element {
  const [state, setState] = createSignal<TermsState>('loading');
  const [terms, setTerms] = createSignal<PublicConsentText | null>(null);

  const report = (next: TermsState) => {
    setState(next);
    props.onState?.(next);
  };

  const read = async () => {
    const api = props.api ?? new ApiClient(props.fetch as Fetch | undefined);
    setTerms(null);
    report('loading');
    try {
      const org = await readHostOrg(api);
      const texts = await api.publicGet<PublicConsentText[]>(
        `/api/public/orgs/${encodeURIComponent(org)}/consent_texts?lang=${encodeURIComponent(lang())}`,
      );
      if (!Array.isArray(texts)) return report('failed');
      const item = texts.find((t) => t.purpose === 'TERMS') ?? null;
      setTerms(item);
      report(item ? 'ready' : 'missing');
    } catch {
      report('failed');
    }
  };

  onMount(() => void read());

  return (
    <Switch>
      <Match when={state() === 'loading'}>
        <p class="xc-sign-in-terms" data-testid="terms-loading">
          {tr('termsLoading')}
        </p>
      </Match>
      <Match when={state() === 'missing'}>
        <p class="xc-sign-in-terms-missing" role="alert" data-testid="terms-missing">
          {tr('consentTextMissing')}
        </p>
      </Match>
      <Match when={state() === 'failed'}>
        <div class="xc-sign-in-terms-failed-box">
          <p class="xc-sign-in-terms-failed" role="alert" data-testid="terms-failed">
            {tr('termsFailed')}
          </p>
          <button type="button" class="xc-sign-in-link" data-testid="terms-retry" onClick={read}>
            {tr('termsRetry')}
          </button>
        </div>
      </Match>
      <Match when={terms()}>
        {(t) => (
          <div class="xc-sign-in-terms-box">
            <p class="xc-sign-in-terms-version" data-testid="terms-version">
              {tr('termsVersion', { version: t().version })}
            </p>
            <div class="xc-sign-in-terms-body" data-testid="terms-body">
              {t().body}
            </div>
          </div>
        )}
      </Match>
    </Switch>
  );
}
