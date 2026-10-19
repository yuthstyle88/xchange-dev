// P01 v2 band headings (rate-display.md §9.4): the currency heading, then one heading per band (or "all amounts").
// SKELETON STUB (feat/fe-rateboard): the function ticket replaces this body; the props and export stay.
import type { JSX } from 'solid-js';
import type { BandHeading } from './board-model';

export interface BandHeadingsProps {
  headings: BandHeading[];
  singleColumn: boolean;
}

export function BandHeadings(_props: BandHeadingsProps): JSX.Element {
  return null;
}
