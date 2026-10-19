// C09: a saved plan's delete button, plannerDraftDelete `POST /api/app/planner-drafts/{id}/delete`
// (design customer-app.md §4.5, §9.1). SKELETON STUB (feat/fe-customer): the function ticket replaces this
// body; the props and export stay.
import type { JSX } from 'solid-js';
import type { ApiClient } from '../../../shared/api/client';
import type { PlannerDraftDto } from '../../../shared/api/generated/my_planner_drafts';

export interface PlanDeleteProps {
  draft: PlannerDraftDto;
  /** The journey screen's client; the write goes through `keyedWrite`. */
  api: ApiClient;
  /** Called with the draft's id once the server has deleted it. */
  onDeleted: (id: string) => void;
}

export function PlanDelete(_props: PlanDeleteProps): JSX.Element {
  return null;
}
