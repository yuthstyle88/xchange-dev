// C14: the TERMS text a visitor accepts on sign-up, from the public consent texts
// (design customer-identity-consent.md `:163`; section-b-gaps.md S6; BE contract #3409:
// `GET /api/public/orgs/{org}/consent_texts?lang=` answers `[{purpose, version, body}]`).
// SKELETON (feat/fe-customer-2): today's static terms line, moved out of SignUp.tsx unchanged; the function
// ticket reads the published text. The props and export stay.
import type { JSX } from 'solid-js';
import { screenT } from '../../../shared/i18n';
import * as table from './i18n';

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

export function TermsText(_props: TermsTextProps): JSX.Element {
  return <p class="xc-sign-in-terms">{tr('terms')}</p>;
}
