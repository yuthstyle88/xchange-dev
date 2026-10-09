// Stub frame store for DK-C0 (#613).
// Replaced by DK-C (#539), whose save follows #551.
import { createSignal } from 'solid-js';
import { canGo, type FrameStore, type StepIndex } from './steps';

export function createFrameStore(present: readonly StepIndex[]): FrameStore {
  const initial = (present.length > 0 ? Math.min(...present) : 0) as StepIndex;
  const [step, setStep] = createSignal<StepIndex>(initial);

  function go(to: StepIndex): void {
    if (canGo(step(), to, present)) {
      setStep(to);
    }
  }

  return { step, go };
}
