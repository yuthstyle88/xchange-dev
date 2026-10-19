// C07 customer profile and consent screen (design customer-identity-consent.md §4.3, §4.4, K5).
// SKELETON STUB (feat/fe-customer): the function ticket replaces this body; the props and export stay.
import type { JSX } from 'solid-js';
import type { ApiClient } from '../../../shared/api/client';
import './profile.css';

export interface ProfileScreenProps {
  /** Injected so tests mock at the fetch level; defaults to `new ApiClient()`. */
  client?: ApiClient;
}

export function ProfileScreen(_props: ProfileScreenProps = {}): JSX.Element {
  return <div class="xc-profile" />;
}
