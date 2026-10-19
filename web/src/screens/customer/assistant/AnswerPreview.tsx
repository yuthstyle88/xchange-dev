// C12: what the assistant understood — the intent's preview to confirm as a planner draft, or the link its kind
// leads to (design customer-app.md P11 `:71`; section-b-gaps.md S4; BE contract #3407: the link is derived on the
// client from `intent.kind`, the back end sends no URL).
// SKELETON STUB (feat/fe-customer-2): the function ticket replaces this body; the props and export stay.
import type { JSX } from 'solid-js';
import type { ApiClient } from '../../../shared/api/client';
import type { AssistantAskAnswerDto } from '../../../shared/api/generated/assistant_ask';
import type { PlannerPreviewAnswer } from '../../../shared/api/generated/planner_preview';

/** `AssistantIntentDto` as #3407 states it; optional fields are omitted when absent. */
export interface AssistantIntent {
  kind: 'QUOTE' | 'QUEUE' | 'ALERT' | 'BRANCH' | 'UNKNOWN';
  currency?: string;
  customerSide?: string;
  quantity?: string;
  budgetThb?: string;
  branch?: string;
}

/** The fields #3407 adds to `AssistantAskAnswerDto`. Optional until the generated type has them. */
export interface AssistantAnswerExtras {
  intent?: AssistantIntent;
  preview?: PlannerPreviewAnswer | null;
}

export interface AnswerPreviewProps {
  answer: AssistantAskAnswerDto & AssistantAnswerExtras;
  /** The screen's client; the planner-draft write goes through `keyedWrite`. */
  api: ApiClient;
  /** The screen's branch, used when the intent names none. */
  branch?: string;
}

export function AnswerPreview(_props: AnswerPreviewProps): JSX.Element {
  return null;
}
