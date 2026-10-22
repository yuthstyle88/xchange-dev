// C14: the TERMS text a visitor accepts on sign-up, from the public consent texts
// (design customer-identity-consent.md `:163`; section-b-gaps.md S6; BE contract #3409:
// `GET /api/public/orgs/{org}/consent_texts?lang=` answers `[{purpose, version, body}]`).
import { createSignal, onMount, Show, type JSX } from 'solid-js';
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

export interface TermsTextProps {
  /** The sign-up screen's fetch, so tests mock at the fetch level. */
  fetch?: typeof fetch;
  api?: Pick<ApiClient, 'publicGet'>;
}

async function loadTerms(api: Pick<ApiClient, 'publicGet'>, currentLang: string): Promise<PublicConsentText | null> {
  try {
    const org = await readHostOrg(api);
    const texts = await api.publicGet<PublicConsentText[]>(
      `/api/public/orgs/${encodeURIComponent(org)}/consent_texts?lang=${encodeURIComponent(currentLang)}`,
    );
    if (!Array.isArray(texts)) return null;

    const termsItem = texts.find((item) => item.purpose === 'TERMS');
    return termsItem ?? null;
  } catch {
    return null;
  }
}

export function TermsText(props: TermsTextProps = {}): JSX.Element {
  const [terms, setTerms] = createSignal<PublicConsentText | null>(null);

  onMount(() => {
    const api = props.api ?? new ApiClient(props.fetch as Fetch | undefined);
    void loadTerms(api, lang()).then(setTerms);
  });

  return (
    <Show
      when={terms()}
      fallback={
        <p class="xc-sign-in-terms">
          {tr('terms')}
        </p>
      }
    >
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
    </Show>
  );
}
