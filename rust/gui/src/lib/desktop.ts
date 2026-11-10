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

export interface BackendStatus {
  running: boolean;
  /** "wsl", "distro" (setup needed) or a message */
  problem: string | null;
  windows: boolean;
  distro: string;
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
