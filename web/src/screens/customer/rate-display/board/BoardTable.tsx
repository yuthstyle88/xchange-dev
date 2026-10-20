// P01 v2 rates table (rate-display.md §9.2): headings in <thead>, one <tr> per row in <tbody>; sizes the rows to fit
// the canvas (--n, --fs) and runs the attract spotlight and wave when `attract` is on.
import { createEffect, createMemo, For, onCleanup, onMount, Show, type JSX } from 'solid-js';
import type { BoardModel, BoardRow } from './board-model';
import { BandHeadings } from './BandHeadings';
import { BoardRowView } from './BoardRowView';
import './board-table.css';

export interface BoardTableProps {
  model: BoardModel;
  attract: boolean;
}

export function BoardTable(props: BoardTableProps): JSX.Element {
  let tableEl: HTMLTableElement | undefined;
  let tbodyEl: HTMLTableSectionElement | undefined;
  let spotEl: HTMLTableCaptionElement | undefined;
  let lit = -1;

  const rowKeys = createMemo(
    () => props.model.rows.map((r) => r.key),
    [],
    { equals: (a, b) => a.length === b.length && a.every((k, i) => k === b[i]) },
  );

  const rowsByKey = createMemo(() => {
    const map = new Map<string, BoardRow>();
    for (const r of props.model.rows) {
      map.set(r.key, r);
    }
    return map;
  });

  const size = () => {
    if (!tableEl) return;
    const n = props.model.rows.length || 1;
    tableEl.style.setProperty('--n', String(n));
    const h = (tbodyEl?.clientHeight || tableEl.clientHeight) / n || 40;
    tableEl.style.setProperty('--fs', `${Math.max(14, Math.min(34, h * 0.47))}px`);
  };

  const place = () => {
    if (!tbodyEl || !spotEl || lit < 0) return;
    const row = tbodyEl.querySelectorAll<HTMLElement>('tr')[lit];
    if (!row) return;
    spotEl.style.transform = `translateY(${row.offsetTop}px)`;
    spotEl.style.height = `${row.offsetHeight}px`;
  };

  const clearSpotlight = () => {
    tbodyEl?.querySelectorAll<HTMLElement>('tr.xc-rb-lit').forEach((r) => r.classList.remove('xc-rb-lit'));
    spotEl?.classList.remove('xc-rb-spot-on', 'xc-rb-spot-glint');
    lit = -1;
  };

  const isIdle = () => !props.attract || document.hidden;

  const walk = () => {
    if (!tbodyEl) return;
    const all = tbodyEl.querySelectorAll<HTMLElement>('tr');
    all[lit]?.classList.remove('xc-rb-lit');
    if (isIdle() || all.length === 0) {
      spotEl?.classList.remove('xc-rb-spot-on', 'xc-rb-spot-glint');
      return;
    }
    lit = (lit + 1) % all.length;
    all[lit]?.classList.add('xc-rb-lit');
    place();
    if (spotEl) {
      spotEl.classList.add('xc-rb-spot-on');
      spotEl.classList.remove('xc-rb-spot-glint');
      void spotEl.offsetWidth;
      spotEl.classList.add('xc-rb-spot-glint');
    }
  };

  const wave = () => {
    if (isIdle() || !tbodyEl) return;
    const all = Array.from(tbodyEl.querySelectorAll<HTMLElement>('tr'));
    all.forEach((row) =>
      row.querySelectorAll<HTMLElement>('.xc-rb-ch, .xc-rb-price').forEach((c) => c.classList.remove('xc-rb-flip')),
    );
    void tbodyEl.offsetWidth;
    all.forEach((row, r) => {
      row.querySelectorAll<HTMLElement>('.xc-rb-price').forEach((price) => {
        const chars = price.querySelectorAll<HTMLElement>('.xc-rb-ch');
        if (chars.length > 0) {
          chars.forEach((c, i) => {
            c.style.setProperty('--d', `${r * 55 + i * 30}ms`);
            c.classList.add('xc-rb-flip');
          });
        } else {
          price.style.setProperty('--d', `${r * 55}ms`);
          price.classList.add('xc-rb-flip');
        }
      });
    });
  };

  createEffect(() => {
    void props.model.rows.length;
    size();
  });

  createEffect(() => {
    if (!props.attract) {
      clearSpotlight();
      return;
    }
    const walkTimer = setInterval(walk, 1900);
    const waveTimer = setInterval(wave, 70000);
    onCleanup(() => {
      clearInterval(walkTimer);
      clearInterval(waveTimer);
    });
  });

  onMount(() => {
    size();
    const onResize = () => {
      size();
      place();
    };
    window.addEventListener('resize', onResize);
    onCleanup(() => window.removeEventListener('resize', onResize));
  });

  return (
    <table ref={tableEl} class="xc-rb-table">
      <Show when={props.attract}>
        <caption ref={spotEl} class="xc-rb-spot" aria-hidden="true" />
      </Show>
      <thead>
        <BandHeadings headings={props.model.headings} singleColumn={props.model.singleColumn} />
      </thead>
      <tbody ref={tbodyEl} class="xc-rb-tbody">
        <For each={rowKeys()}>
          {(key) => {
            const row = createMemo(() => rowsByKey().get(key)!);
            return <BoardRowView row={row()} singleColumn={props.model.singleColumn} />;
          }}
        </For>
      </tbody>
    </table>
  );
}
