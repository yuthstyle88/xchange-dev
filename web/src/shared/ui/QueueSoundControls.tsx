import type { JSX } from 'solid-js';
import { t } from '../i18n';
import { hint, soundOn, testSound, toggleSound } from '../queue/queue-sound';

export function QueueSoundControls(): JSX.Element {
  return (
    <div class="queue-sound-controls">
      <button
        type="button"
        aria-pressed={soundOn() ? 'true' : 'false'}
        onClick={() => void toggleSound()}
      >
        {soundOn() ? t('queue.sound.hintOn') : t('queue.sound.enable')}
      </button>
      <button type="button" onClick={() => void testSound()}>
        {t('queue.sound.test')}
      </button>
      <span>{hint()}</span>
    </div>
  );
}
