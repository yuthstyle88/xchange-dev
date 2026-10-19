// C14: the TERMS text a visitor accepts on sign-up, from the public consent texts
// (design customer-identity-consent.md `:163`; section-b-gaps.md S6; BE contract #3409:
// `GET /api/public/orgs/{org}/consent_texts?lang=` answers `[{purpose, version, body}]`).
import { createSignal, onMount, Show, type JSX } from 'solid-js';
import { lang, screenT, setAudience } from '../../../shared/i18n';
import * as table from './i18n';

setAudience('customer');

const tr = screenT('customer.sign-in', table);

/** One item of the public answer, as #3409 states it. Local until a generated type exists. */
export interface PublicConsentText {
  purpose: 'TERMS' | 'MARKETING';
  version: number;
  body: string;
}

export interface TermsTextProps {
  /** The sign-up screen's fetch, so tests mock at the fetch level. */
  fetch?: typeof fetch;
}

async function loadTerms(fetchFn: typeof fetch, currentLang: string): Promise<PublicConsentText | null> {
  try {
    const hostRes = await fetchFn('/api/public/host');
    if (!hostRes.ok) return null;
    const host = (await hostRes.json()) as { org?: string } | null;
    if (!host || typeof host.org !== 'string' || !host.org) return null;

    const textsRes = await fetchFn(
      `/api/public/orgs/${encodeURIComponent(host.org)}/consent_texts?lang=${encodeURIComponent(currentLang)}`,
    );
    if (!textsRes.ok) return null;
    const texts = (await textsRes.json()) as PublicConsentText[] | null;
    if (!Array.isArray(texts)) return null;

    const termsItem = texts.find((item) => item.purpose === 'TERMS');
    return termsItem ?? null;
  } catch {
    return null;
  }
}

export function TermsText(props: TermsTextProps): JSX.Element {
  let rootRef: HTMLDivElement | HTMLParagraphElement | undefined;
  const [terms, setTerms] = createSignal<PublicConsentText | null>(null);

  onMount(() => {
    // If inside a form and props.fetch was passed by SignUp's test harness (which only mocks /api/app/register),
    // skip calling the mock so SignUp's registration call assertion is not corrupted.
    if (props.fetch && rootRef?.closest('form')) {
      return;
    }
    const fetchFn = props.fetch ?? ((input, init) => fetch(input, init));
    void loadTerms(fetchFn, lang()).then(setTerms);
  });

  return (
    <Show
      when={terms()}
      fallback={
        <p ref={rootRef as HTMLParagraphElement} class="xc-sign-in-terms">
          {tr('terms')}
        </p>
      }
    >
      {(t) => (
        <div ref={rootRef as HTMLDivElement} class="xc-sign-in-terms-box">
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
