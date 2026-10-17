import { ApiClient } from '../../../shared/api/client';
import { keyedWrite } from '../../../shared/api/keyed-write';
import type {
  MyVerificationCaseDetail,
  MyVerificationDocumentMeta,
  MyVerificationViewAnswer,
} from '../../../shared/api/generated/my_verification';
import type {
  VerificationCaseOpenInput,
  VerificationCaseOpenOutput,
} from '../../../shared/api/generated/verification_case_open';
import type {
  VerificationCaseSubmitInput,
  VerificationCaseSubmitOutput,
} from '../../../shared/api/generated/verification_case_submit';
import type {
  VerificationDocumentRemoveInput,
  VerificationDocumentRemoveOutput,
} from '../../../shared/api/generated/verification_document_remove';
import type {
  VerificationMyDocumentLinkInput,
  VerificationMyDocumentLinkOutput,
} from '../../../shared/api/generated/verification_my_document_link';

export type {
  MyVerificationCaseDetail,
  MyVerificationDocumentMeta,
  MyVerificationViewAnswer,
  VerificationCaseOpenInput,
  VerificationCaseOpenOutput,
  VerificationCaseSubmitInput,
  VerificationCaseSubmitOutput,
  VerificationDocumentRemoveInput,
  VerificationDocumentRemoveOutput,
  VerificationMyDocumentLinkInput,
  VerificationMyDocumentLinkOutput,
};

export type VerificationDocumentType = 'PASSPORT' | 'NATIONAL_ID';

export interface VerificationDocumentUploadOutput {
  id: string;
  caseId: string;
  documentType: string;
  mime: string;
  sizeBytes: number;
  uploadedAt: string;
  accessClass: string;
}

export interface MyProfileView {
  displayName?: string;
  permissions: readonly string[];
}

const api = new ApiClient();

const openCaseWriter = keyedWrite<VerificationCaseOpenInput, VerificationCaseOpenOutput>(
  api,
  'POST',
  '/api/app/verification/cases',
);

const removeDocumentWriter = keyedWrite<
  VerificationDocumentRemoveInput,
  VerificationDocumentRemoveOutput
>(api, 'POST', (body) => `/api/app/verification/documents/${encodeURIComponent(body.id)}/remove`);

const submitCaseWriter = keyedWrite<VerificationCaseSubmitInput, VerificationCaseSubmitOutput>(
  api,
  'POST',
  (body) => `/api/app/verification/cases/${encodeURIComponent(body.id)}/submit`,
);

let keyCounter = 0;

export function newIdempotencyKey(): string {
  keyCounter += 1;
  return `idem-${Date.now().toString(36)}-${keyCounter.toString(36)}-${Math.random()
    .toString(36)
    .slice(2, 10)}`;
}

export async function fetchMyProfile(): Promise<MyProfileView> {
  return api.viewBare<MyProfileView>('my_profile');
}

export async function fetchMyVerification(): Promise<MyVerificationViewAnswer> {
  return api.viewBare<MyVerificationViewAnswer>('my_verification');
}

export async function openVerificationCase(
  input: VerificationCaseOpenInput = { reason: null },
): Promise<VerificationCaseOpenOutput> {
  return openCaseWriter.send(input);
}

export async function uploadVerificationDocument(
  caseId: string,
  documentType: VerificationDocumentType | string,
  file: File,
  idempotencyKey: string,
): Promise<VerificationDocumentUploadOutput> {
  const form = new FormData();
  form.append('documentType', documentType);
  form.append('file', file);
  return api.postForm<VerificationDocumentUploadOutput>(
    `/api/app/verification/cases/${encodeURIComponent(caseId)}/documents`,
    form,
    idempotencyKey,
  );
}

export async function removeVerificationDocument(
  input: VerificationDocumentRemoveInput,
): Promise<VerificationDocumentRemoveOutput> {
  return removeDocumentWriter.send(input);
}

export async function submitVerificationCase(
  input: VerificationCaseSubmitInput,
): Promise<VerificationDocumentRemoveOutput & VerificationCaseSubmitOutput> {
  return submitCaseWriter.send(input) as Promise<
    VerificationDocumentRemoveOutput & VerificationCaseSubmitOutput
  >;
}

export async function requestMyDocumentLink(
  input: VerificationMyDocumentLinkInput,
): Promise<VerificationMyDocumentLinkOutput> {
  return api.post<VerificationMyDocumentLinkInput, VerificationMyDocumentLinkOutput>(
    `/api/app/verification/documents/${encodeURIComponent(input.id)}/link`,
    input,
  );
}
