// V2-shell (#460): admin entry point

import { render } from 'solid-js/web';
import { ApiClient } from '../../shared/api/client';
import { AdminShell } from './AdminShell';
import { menus } from './menus';
import './admin.css';

const appEl = document.getElementById('app');
if (appEl) {
  const api = new ApiClient();
  render(() => <AdminShell api={api} menus={menus} />, appEl);
}
