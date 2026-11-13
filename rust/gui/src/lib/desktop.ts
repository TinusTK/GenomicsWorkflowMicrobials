// Things only the desktop app can do (open a folder in Explorer/Finder, set up
// WSL and Docker). In a browser the folder path is copied instead.

import { toasts } from './toast.svelte';

type Invoker = (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;

function invoker(): Invoker | null {
  const t = (window as unknown as { __TAURI_INTERNALS__?: { invoke: Invoker } }).__TAURI_INTERNALS__;
  return t ? t.invoke : null;
}

export const isDesktop = () => invoker() !== null;

export async function invoke<T = unknown>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const i = invoker();
  if (!i) throw new Error('Only in the desktop app');
  return (await i(cmd, args)) as T;
}

/**
 * What Colima's Linux machine gets on a Mac: half the threads, and all the
 * memory but 4 GB, which stay for the Mac itself (Colima's own default, 2 CPUs
 * and 2 GB, is too little).
 */
export function colimaSize(hw: { cpus: number; memory_gb: number } | null) {
  return {
    cpus: Math.max(2, Math.floor((hw?.cpus ?? 4) / 2)),
    memory: Math.max(2, Math.round(hw?.memory_gb ?? 8) - 4),
  };
}

export interface BackendStatus {
  running: boolean;
  /** "wsl", "distro" (setup needed) or a message */
  problem: string | null;
  windows: boolean;
  distro: string;
  /** closing the window keeps the app in the system tray */
  tray?: boolean;
}

export async function backendStatus(): Promise<BackendStatus | null> {
  return isDesktop() ? invoke<BackendStatus>('backend_status').catch(() => null) : null;
}

export async function openFolder(path: string) {
  if (isDesktop()) {
    try {
      await invoke('open_folder', { path });
    } catch (e) {
      toasts.show(`Could not open the folder: ${e}`, 'error');
    }
    return;
  }
  try {
    await navigator.clipboard.writeText(path);
    toasts.show('Folder path copied: paste it into your file manager.', 'info');
  } catch {
    toasts.show(path, 'info', undefined, 12000);
  }
}

/**
 * Save a text file where the user chooses (desktop app), or download it (in a
 * browser). Returns where it went; null when the user cancelled.
 */
export async function saveTextFile(name: string, text: string): Promise<string | null> {
  if (isDesktop()) return invoke<string | null>('save_text_file', { name, text });
  const link = document.createElement('a');
  link.href = URL.createObjectURL(new Blob([text], { type: 'text/plain' }));
  link.download = name;
  link.click();
  URL.revokeObjectURL(link.href);
  return 'your downloads folder';
}

/** Open a web page or a new email outside the app (the browser or mail program). */
export async function openLink(url: string) {
  if (isDesktop()) {
    try {
      await invoke('open_url', { url });
    } catch (e) {
      toasts.show(`Could not open ${url}: ${e}`, 'error');
    }
    return;
  }
  window.open(url, '_blank', 'noopener');
}
