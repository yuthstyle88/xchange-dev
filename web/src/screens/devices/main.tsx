import { Show } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiClient } from '../../shared/api/client';
import { sharedSession } from '../../shared/session/session';
import { pageBranch } from '../../shared/session/page-branch';
import { DevicesScreen } from './DevicesScreen';

function downloadFile(name: string, blob: Blob): void {
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = name;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}

const appEl = document.getElementById('app');
if (appEl) {
  const branch = pageBranch(sharedSession());
  const api = new ApiClient();

  render(
    () => (
      <Show when={branch()} keyed>
        {(b) => (
          <DevicesScreen
            api={api}
            branch={b}
            download={downloadFile}
          />
        )}
      </Show>
    ),
    appEl,
  );
}
