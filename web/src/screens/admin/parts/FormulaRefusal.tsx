// Pricing formula FE T5 (docs/design/blueprint/pricing-formula-web.md §2, §3 R1–R2): a refused save's failing
// prices, one row per line.
import { For, Show, type JSX } from 'solid-js';
import { lang } from '../../../shared/i18n';
import { en, th } from './formula-i18n';
import { failureText, type SpreadFailureLine } from './spread-failure';
import './formula.css';

export interface FormulaRefusalProps {
  lines: () => readonly SpreadFailureLine[];
}

export function FormulaRefusal(props: FormulaRefusalProps): JSX.Element {
  const t = () => (lang() === 'en' ? en : th);

  return (
    <Show when={props.lines().length > 0}>
      <div class="formula-refusal" role="alert">
        <h3>{t().refusalTitle}</h3>
        <table class="formula-refusal-table">
          <thead>
            <tr>
              <th>{t().colBranch}</th>
              <th>{t().colCurrency}</th>
              <th>{t().colTier}</th>
              <th>{t().colMode}</th>
              <th>{t().colBuy}</th>
              <th>{t().colSell}</th>
              <th>{t().colProblem}</th>
            </tr>
          </thead>
          <tbody>
            <For each={props.lines()}>
              {(line) => (
                <tr>
                  <td>{line.branch}</td>
                  <td>{line.currency}</td>
                  <td>{line.tier !== null ? line.tier : t().dash}</td>
                  <td>{line.follow ? t().modeFollow : t().modeAuto}</td>
                  <td>{line.buy !== null ? line.buy : t().dash}</td>
                  <td>{line.sell !== null ? line.sell : t().dash}</td>
                  <td>{failureText(line)}</td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>
    </Show>
  );
}
