// Things only the desktop app can do (open a folder in Explorer/Finder). In a
// browser the path is copied instead, so it can be pasted into the file manager.

import { toasts } from './toast.svelte';

type Invoke = (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;

function invoker(): Invoke | null {
  const t = (window as unknown as { __TAURI_INTERNALS__?: { invoke: Invoke } }).__TAURI_INTERNALS__;
  return t ? t.invoke : null;
}

export const isDesktop = () => invoker() !== null;

export async function openFolder(path: string) {
  const invoke = invoker();
  if (invoke) {
    try {
      await invoke('open_folder', { path });
      return;
    } catch (e) {
      toasts.show(`Could not open the folder: ${e}`, 'error');
      return;
    }
  }
  try {
    await navigator.clipboard.writeText(path);
    toasts.show('Folder path copied: paste it into your file manager.', 'info');
  } catch {
    toasts.show(path, 'info', undefined, 12000);
  }
}
