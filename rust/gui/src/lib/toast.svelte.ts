// Short messages in the corner ("Saved as version 3"), with an optional Undo.

export interface Toast {
  id: number;
  text: string;
  kind: 'ok' | 'error' | 'info';
  action?: { label: string; run: () => void };
}

class Toasts {
  list = $state<Toast[]>([]);
  #next = 1;

  show(text: string, kind: Toast['kind'] = 'ok', action?: Toast['action'], ms = 5000) {
    const id = this.#next++;
    this.list.push({ id, text, kind, action });
    setTimeout(() => this.close(id), kind === 'error' ? ms * 2 : ms);
  }

  close(id: number) {
    this.list = this.list.filter((t) => t.id !== id);
  }
}

export const toasts = new Toasts();

export function errorText(e: unknown) {
  return e instanceof Error ? e.message : String(e);
}
