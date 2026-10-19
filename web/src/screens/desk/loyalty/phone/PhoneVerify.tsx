// B03 desk phone capture + SMS code (FE-desk skeleton stub, #564; design membership-points.md §6.3 CU-P1; commands
// api/src/presentation/http/v3/commands/customer_phone_start.rs:29-62 and customer_phone_confirm.rs:29-70, which export
// no TS type, so the types are here). Filled by its own ticket.
import type { JSX } from 'solid-js';
import type { ApiClient } from '../../../../shared/api/client';

/** `POST /api/customers/{id}/phone-verifications` body (`customer_phone_start.rs:29-33`). */
export interface PhoneStartInput {
  id: string;
  branch: string;
  phone: string;
}
/** Its answer (`customer_phone_start.rs:37-40`). */
export interface PhoneStartOutput {
  challengeId: string;
  expiresAt: string;
}
/** `POST /api/customers/{id}/phone-verifications/{challengeId}/confirm` body (`customer_phone_confirm.rs:29-35`). */
export interface PhoneConfirmInput {
  id: string;
  challengeId: string;
  branch: string;
  code: string;
}
/** Its answer (`customer_phone_confirm.rs:39-42`). */
export interface PhoneConfirmOutput {
  verified: boolean;
  identityId: string;
}

export interface PhoneVerifyProps {
  api: ApiClient;
  branch: string;
  customerId: string;
  /** Called once the server answers `verified: true`. */
  onVerified(): void;
}

export function PhoneVerify(_props: PhoneVerifyProps): JSX.Element {
  return null;
}
