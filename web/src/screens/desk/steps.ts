import type { Accessor, JSX } from 'solid-js';

export type StepIndex = 0 | 1 | 2 | 3 | 4;
export type FlowPhase = 'quote' | 'check' | 'confirm';

/** Blueprint §03 B03 /desk/exchange: "quote → ตรวจ → ยืนยันรายการ" — keep this citation in the source (test F3). */
export const DESK_FLOW: readonly { index: StepIndex; phase: FlowPhase }[] = [
  { index: 0, phase: 'quote' },
  { index: 1, phase: 'check' },
  { index: 2, phase: 'check' },
  { index: 3, phase: 'check' },
  { index: 4, phase: 'confirm' },
] as const;

export interface FrameStore {
  step: Accessor<StepIndex>;
  go(to: StepIndex): void;
}

export interface DeskStep<S extends FrameStore = FrameStore> {
  index: StepIndex;
  label: () => string;
  component: (p: { store: S }) => JSX.Element;
}

const STEP_PATH_RE = /^\.\/steps\/[a-z][a-z0-9-]*\/step\.tsx$/;

export function collectSteps<S extends FrameStore = FrameStore>(
  glob: Record<string, unknown>,
): DeskStep<S>[] {
  const byIndex = new Map<StepIndex, { step: DeskStep<S>; path: string }>();

  for (const [path, mod] of Object.entries(glob)) {
    if (!STEP_PATH_RE.test(path)) {
      throw new Error(`Invalid step path: "${path}"`);
    }

    const m = mod as { step?: DeskStep<S> } | null | undefined;
    if (!m || typeof m !== 'object' || !m.step) {
      throw new Error(`Missing step export in "${path}"`);
    }

    const idx = m.step.index;
    if (typeof idx !== 'number' || !DESK_FLOW.some((f) => f.index === idx)) {
      throw new Error(`Invalid step index ${idx} in "${path}"`);
    }

    const existing = byIndex.get(idx);
    if (existing) {
      throw new Error(
        `Duplicate step index ${idx} in "${existing.path}" and "${path}"`,
      );
    }

    byIndex.set(idx, { step: m.step, path });
  }

  return [...byIndex.values()]
    .map((v) => v.step)
    .sort((a, b) => a.index - b.index);
}

export function nextIndex(
  current: StepIndex,
  present: readonly StepIndex[],
): StepIndex | undefined {
  const candidates = present.filter((i) => i > current).sort((a, b) => a - b);
  return candidates.length > 0 ? candidates[0] : undefined;
}

export function prevIndex(
  current: StepIndex,
  present: readonly StepIndex[],
): StepIndex | undefined {
  const candidates = present.filter((i) => i < current).sort((a, b) => b - a);
  return candidates.length > 0 ? candidates[0] : undefined;
}

export function canGo(
  from: StepIndex,
  to: StepIndex,
  present: readonly StepIndex[],
): boolean {
  if (!present.includes(to)) return false;
  if (to === from) return false;
  if (to < from) return true;
  return to === nextIndex(from, present);
}
