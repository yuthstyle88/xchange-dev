// C09: a saved plan's delete button, plannerDraftDelete `POST /api/app/planner-drafts/{id}/delete`
// (design customer-app.md §4.5, §9.1). SKELETON STUB (feat/fe-customer): the function ticket replaces this
// body; the props and export stay.
import { createSignal, Show, type JSX } from 'solid-js';
import { ApiError, type ApiClient } from '../../../shared/api/client';
import { keyedWrite } from '../../../shared/api/keyed-write';
import type { PlannerDraftDto } from '../../../shared/api/generated/my_planner_drafts';
import type {
  PlannerDraftDeleteInput,
  PlannerDraftDeleteOutput,
} from '../../../shared/api/generated/planner_draft_delete';
import { screenT } from '../../../shared/i18n';
import { th, en, zh } from './i18n';

export interface PlanDeleteProps {
  draft: PlannerDraftDto;
  /** The journey screen's client; the write goes through `keyedWrite`. */
  api: ApiClient;
  /** Called with the draft's id once the server has deleted it. */
  onDeleted: (id: string) => void;
}

export function PlanDelete(props: PlanDeleteProps): JSX.Element {
  const t = screenT('customer.journey', { th, en, zh });
  const writer = keyedWrite<PlannerDraftDeleteInput, PlannerDraftDeleteOutput>(
    props.api,
    'POST',
    (b) => `/api/app/planner-drafts/${b.id}/delete`,
  );
  const [busy, setBusy] = createSignal(false);
  const [failed, setFailed] = createSignal(false);

  const handleDelete = () => {
    if (busy()) return;
    setBusy(true);
    setFailed(false);
    const id = props.draft.id;
    void writer
      .send({ id })
      .then(() => {
        setBusy(false);
        props.onDeleted(id);
      })
      .catch((err: unknown) => {
        setBusy(false);
        if (err instanceof ApiError && err.status === 404) {
          props.onDeleted(id);
          return;
        }
        setFailed(true);
      });
  };

  return (
    <div class="xc-plan-delete">
      <button
        type="button"
        class="xc-btn xc-btn-outline-danger"
        data-action="delete-plan"
        disabled={busy()}
        onClick={handleDelete}
      >
        {t('deletePlan')}
      </button>
      <Show when={failed()}>
        <span data-testid="plan-delete-error">{t('deletePlanFailed')}</span>
      </Show>
    </div>
  );
}
