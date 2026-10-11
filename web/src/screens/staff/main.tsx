import { render } from 'solid-js/web';
import { ApiClient } from '../../shared/api/client';
import { AppShell } from '../../shared/ui/AppShell';
import { StaffPage } from './StaffPage';
import { cards } from './cards';
import './staff.css';

void AppShell;

const appEl = document.getElementById('app');
if (appEl) {
  const api = new ApiClient();
  const navigate = (url: string) => {
    window.location.assign(url);
  };
  render(() => <StaffPage api={api} cards={cards} navigate={navigate} />, appEl);
}
