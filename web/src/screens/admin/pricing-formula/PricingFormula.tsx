// Pricing formula FE T6 (docs/design/blueprint/pricing-formula-web.md §1, §3 F1–F11): the organisation formula
// screen — the editor, the live preview (scope ORGANISATION), which formula each branch follows, and the save.
import { createEffect, createSignal, createUniqueId, For, Show, type JSX } from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import type { PricingFormulaData } from '../../../shared/api/generated/pricing_formula';
import type { FormulaNotCheckedDto } from '../../../shared/api/generated/pricing_formula_preview';
import type {
  PricingFormulaSaveInput,
  PricingFormulaSaveOutput,
} from '../../../shared/api/generated/pricing_formula_save';
import { keyedWrite } from '../../../shared/api/keyed-write';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import type { AdminMenuProps } from '../menus';
import { FormulaEditor } from '../parts/FormulaEditor';
import * as formulaTable from '../parts/formula-i18n';
import { createFormulaPreview, ROUTE_PRICING_FORMULA_SAVE } from '../parts/formula-preview';
import { FormulaPreviewTable } from '../parts/FormulaPreviewTable';
import { FormulaRefusal } from '../parts/FormulaRefusal';
import { spreadRefusal, type SpreadFailureLine } from '../parts/spread-failure';
import { toast } from '../parts/toast';
import * as table from './i18n';
import './pricing-formula.css';

const t = screenT('admin.pricing-formula', table);
const tf = formulaTable.formulaT;

export function branchLabel(id: string): string {
  return id.length > 12 ? `${id.slice(0, 8)}…` : id;
}

export function PricingFormula(props: AdminMenuProps): JSX.Element {
  const api = new ApiClient();
  const reasonId = createUniqueId();
  const writer = keyedWrite<PricingFormulaSaveInput & { branch: string }, PricingFormulaSaveOutput>(
    api,
    'POST',
    ROUTE_PRICING_FORMULA_SAVE,
  );

  const [viewData, setViewData] = createSignal<PricingFormulaData | null>(null);
  const [loadError, setLoadError] = createSignal<string | null>(null);
  const [buy, setBuy] = createSignal('');
  const [sell, setSell] = createSignal('');
  const [reason, setReason] = createSignal('');
  const [saving, setSaving] = createSignal(false);
  const [buyError, setBuyError] = createSignal<string | null>(null);
  const [sellError, setSellError] = createSignal<string | null>(null);
  const [reasonError, setReasonError] = createSignal<string | null>(null);
  const [saveError, setSaveError] = createSignal<string | null>(null);
  const [refusalLines, setRefusalLines] = createSignal<readonly SpreadFailureLine[]>([]);
  const [notChecked, setNotChecked] = createSignal<readonly FormulaNotCheckedDto[]>([]);

  const preview = createFormulaPreview({
    api,
    branch: props.branch,
    scope: 'ORGANISATION',
    buy,
    sell,
    enabled: () => viewData() !== null,
  });

  let loadSeq = 0;
  async function loadView(branch: string): Promise<void> {
    const seq = ++loadSeq;
    setLoadError(null);
    try {
      const answer = await api.view<PricingFormulaData>('pricing_formula', { branch });
      if (seq !== loadSeq || !answer.data) return;
      const d = answer.data;
      const init = d.organisation ?? d.default;
      setBuy(init.buyFormula);
      setSell(init.sellFormula);
      setViewData(d);
    } catch (err: unknown) {
      if (seq !== loadSeq) return;
      setViewData(null);
      setLoadError(wordedError(err));
    }
  }

  createEffect(() => {
    const branch = props.branch();
    void loadView(branch);
  });

  async function onSave(): Promise<void> {
    setBuyError(null);
    setSellError(null);
    setReasonError(null);
    setSaveError(null);
    setRefusalLines([]);

    const trimmedBuy = buy().trim();
    const trimmedSell = sell().trim();
    const trimmedReason = reason().trim();

    let invalid = false;
    if (!trimmedBuy) {
      setBuyError(t('textRequired'));
      invalid = true;
    }
    if (!trimmedSell) {
      setSellError(t('textRequired'));
      invalid = true;
    }
    if (!trimmedReason) {
      setReasonError(t('reasonRequired'));
      invalid = true;
    }
    if (invalid) return;

    setSaving(true);
    try {
      const out = await writer.send({
        branch: props.branch(),
        buyFormula: trimmedBuy,
        sellFormula: trimmedSell,
        reason: trimmedReason,
      });
      toast(t('saved'));
      setNotChecked(out.notChecked);
      setReason('');
      await loadView(props.branch());
    } catch (err: unknown) {
      const lines = spreadRefusal(err);
      if (lines !== null) {
        setRefusalLines(lines);
      } else if (err instanceof ApiError && err.field === 'buyFormula') {
        setBuyError(err.message);
      } else if (err instanceof ApiError && err.field === 'sellFormula') {
        setSellError(err.message);
      } else if (err instanceof ApiError && err.field === 'reason') {
        setReasonError(err.message);
      } else {
        setSaveError(wordedError(err));
      }
    } finally {
      setSaving(false);
    }
  }

  return (
    <div class="pricing-formula-screen">
      <Show
        when={viewData()}
        fallback={
          <Show when={loadError()} fallback={<p>{t('loading')}</p>}>
            {(err) => (
              <div data-testid="formula-load-error">
                <p class="form-error">{err()}</p>
                <button
                  type="button"
                  data-action="formula-retry"
                  onClick={() => void loadView(props.branch())}
                >
                  {t('retry')}
                </button>
              </div>
            )}
          </Show>
        }
      >
        {(d) => (
          <>
            <section class="card pricing-formula-editor">
              <header class="pricing-formula-header" data-testid="formula-current">
                <Show when={d().organisation} fallback={<p>{t('noOrganisation')}</p>}>
                  {(org) => (
                    <p>
                      {t('currentVersion', {
                        version: org().version,
                        approvedBy: org().approvedBy,
                        validFrom: new Date(org().validFrom).toLocaleString('th-TH', {
                          timeZone: 'Asia/Bangkok',
                        }),
                      })}
                    </p>
                  )}
                </Show>
              </header>

              <FormulaEditor
                buy={buy}
                sell={sell}
                onBuy={setBuy}
                onSell={setSell}
                inputs={() => d().inputs.map((i) => i.name)}
                defaults={() => d().default}
                parseError={() => preview.result()?.parseError ?? null}
                disabled={saving}
              />
              <Show when={buyError()}>
                {(msg) => (
                  <p class="form-error" data-field-error="buyFormula">
                    {msg()}
                  </p>
                )}
              </Show>
              <Show when={sellError()}>
                {(msg) => (
                  <p class="form-error" data-field-error="sellFormula">
                    {msg()}
                  </p>
                )}
              </Show>

              <FormulaPreviewTable
                result={preview.result}
                pending={preview.pending}
                error={preview.error}
              />
            </section>

            <section class="card">
              <h3>{t('branchesTitle')}</h3>
              <div class="table-scroll">
                <table class="pricing-formula-branches">
                  <thead>
                    <tr>
                      <th>{t('colBranch')}</th>
                      <th>{t('colSource')}</th>
                      <th>{t('colOwnText')}</th>
                    </tr>
                  </thead>
                  <tbody>
                    <For each={d().branches}>
                      {(b) => (
                        <tr data-branch={b.branch}>
                          <td title={b.branch}>{branchLabel(b.branch)}</td>
                          <td>{tf(`source.${b.source}` as formulaTable.Key)}</td>
                          <td>
                            <Show when={b.own} fallback={tf('dash')}>
                              {(own) => (
                                <>
                                  <code>{own().buyFormula}</code> / <code>{own().sellFormula}</code>
                                </>
                              )}
                            </Show>
                          </td>
                        </tr>
                      )}
                    </For>
                  </tbody>
                </table>
              </div>
            </section>

            <section class="card pricing-formula-save">
              <div class="pricing-formula-reason">
                <label for={reasonId}>{t('reasonLabel')}</label>
                <textarea
                  id={reasonId}
                  data-field="reason"
                  value={reason()}
                  disabled={saving()}
                  onInput={(e) => setReason(e.currentTarget.value)}
                />
                <Show when={reasonError()}>
                  {(msg) => (
                    <p class="form-error" data-field-error="reason">
                      {msg()}
                    </p>
                  )}
                </Show>
              </div>

              <FormulaRefusal lines={refusalLines} />
              <Show when={saveError()}>
                {(msg) => (
                  <p class="form-error" data-field-error="save">
                    {msg()}
                  </p>
                )}
              </Show>

              <Show when={notChecked().length > 0}>
                <div data-testid="formula-not-checked">
                  <p>{tf('notCheckedTitle')}</p>
                  <ul>
                    <For each={notChecked()}>
                      {(item) => <li>{`${item.branch} ${item.currency}`}</li>}
                    </For>
                  </ul>
                </div>
              </Show>

              <div class="pricing-formula-actions">
                <button
                  type="button"
                  class="primary"
                  data-action="formula-save"
                  disabled={saving()}
                  onClick={() => void onSave()}
                >
                  {saving() ? t('saving') : t('save')}
                </button>
              </div>
            </section>
          </>
        )}
      </Show>
    </div>
  );
}
