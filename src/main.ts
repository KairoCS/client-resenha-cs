// Resenha Client — UI mínima (login + status). Toda a lógica pesada fica no Rust.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getVersion } from '@tauri-apps/api/app';

interface Status {
  logged_in: boolean;
  ws_connected: boolean;
  active_match: number | null;
  gsi_path: string | null;
  gsi_listening: boolean;
  gsi_port: number;
  backend_url: string;
  autostart: boolean;
}

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

const loginView = $('loginView');
const statusView = $('statusView');
const statusDot = $('statusDot');
const loginForm = $('loginForm') as unknown as HTMLFormElement;
const loginError = $('loginError');
const loginBtn = $<HTMLButtonElement>('loginBtn');
const backendUrl = $<HTMLInputElement>('backendUrl');
const autostart = $<HTMLInputElement>('autostart');

function render(s: Status) {
  loginView.classList.toggle('hidden', s.logged_in);
  statusView.classList.toggle('hidden', !s.logged_in);

  statusDot.classList.toggle('connected', s.ws_connected && s.active_match === null);
  statusDot.classList.toggle('in-match', s.active_match !== null);
  statusDot.title = s.ws_connected ? 'Conectado' : 'Desconectado';

  const conn = $('connLabel');
  conn.textContent = s.ws_connected ? 'conectado ao servidor' : 'reconectando…';
  conn.className = s.ws_connected ? 'value ok' : 'value bad';

  const match = $('matchLabel');
  if (s.active_match !== null) {
    match.textContent = `partida #${s.active_match} em andamento`;
    match.className = 'value warn';
  } else {
    match.textContent = 'nenhuma partida ativa';
    match.className = 'value';
  }

  const gsi = $('gsiLabel');
  if (!s.gsi_listening) {
    // Sem o servidor local no ar não existe coleta nenhuma — é o erro mais
    // grave possível aqui, então tem prioridade sobre os outros avisos.
    gsi.textContent = `porta ${s.gsi_port} ocupada`;
    gsi.className = 'value bad';
    gsi.title = `Outro programa está usando a porta ${s.gsi_port}. Feche-o e reinicie o Resenha Client.`;
  } else if (s.gsi_path) {
    gsi.textContent = 'pronto';
    gsi.className = 'value ok';
    gsi.title = s.gsi_path;
  } else {
    gsi.textContent = 'CS2 não encontrado';
    gsi.className = 'value bad';
    gsi.title = 'Instale/abra o CS2 uma vez e reinicie o Resenha Client';
  }

  autostart.checked = s.autostart;
  if (!backendUrl.value) backendUrl.value = s.backend_url;
}

async function refresh() {
  try {
    render(await invoke<Status>('get_status'));
  } catch (e) {
    console.error('get_status falhou', e);
  }
}

loginForm.addEventListener('submit', async (ev) => {
  ev.preventDefault();
  loginError.classList.add('hidden');
  loginBtn.disabled = true;
  loginBtn.textContent = 'Conectando…';
  try {
    const codeInput = $<HTMLInputElement>('pairCode');
    await invoke('login', { code: codeInput.value.trim().toUpperCase() });
    codeInput.value = '';
    await refresh();
  } catch (e) {
    loginError.textContent = String(e);
    loginError.classList.remove('hidden');
  } finally {
    loginBtn.disabled = false;
    loginBtn.textContent = 'Conectar';
  }
});

$('saveBackendBtn').addEventListener('click', async () => {
  try {
    await invoke('set_backend_url', { url: backendUrl.value.trim() });
    await refresh();
  } catch (e) {
    loginError.textContent = String(e);
    loginError.classList.remove('hidden');
  }
});

autostart.addEventListener('change', async () => {
  try {
    await invoke('set_autostart', { enabled: autostart.checked });
  } catch (e) {
    console.error('autostart falhou', e);
    autostart.checked = !autostart.checked;
  }
});

$('logoutBtn').addEventListener('click', async () => {
  await invoke('logout');
  await refresh();
});

$('logsBtn').addEventListener('click', () => invoke('open_logs'));

// O Rust emite "status" sempre que algo muda (conexão, partida, login…)
listen<Status>('status', (ev) => render(ev.payload));

// Sessão expirada (refresh token rejeitado) → volta pro login
listen('session-expired', () => {
  loginError.textContent = 'Sua sessão expirou. Entre novamente.';
  loginError.classList.remove('hidden');
  refresh();
});

getVersion().then((v) => ($('versionLabel').textContent = `Resenha Client v${v}`));
refresh();
