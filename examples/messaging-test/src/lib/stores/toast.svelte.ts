/**
 * Toast store using Svelte 5 runes for displaying ephemeral notifications.
 */

export interface Toast {
  id: string;
  title: string;
  body: string;
  link?: string;
  type: 'info' | 'success' | 'warning' | 'error';
  duration: number;
  createdAt: number;
}

// Module-level state using $state rune (Svelte 5)
let toasts = $state<Toast[]>([]);

// Auto-dismiss timeouts
const timeoutMap = new Map<string, ReturnType<typeof setTimeout>>();

function generateId(): string {
  return `toast-${Date.now()}-${Math.random().toString(36).substring(2, 9)}`;
}

function scheduleRemoval(id: string, duration: number): void {
  const timeout = setTimeout(() => {
    toastStore.remove(id);
  }, duration);
  timeoutMap.set(id, timeout);
}

// Exported store object
export const toastStore = {
  // Getter for reactive state
  get toasts() {
    return toasts;
  },

  add(
    title: string,
    body: string = '',
    link?: string,
    options?: {
      type?: Toast['type'];
      duration?: number;
    }
  ): string {
    const id = generateId();
    const toast: Toast = {
      id,
      title,
      body,
      link,
      type: options?.type || 'info',
      duration: options?.duration || 5000,
      createdAt: Date.now(),
    };

    // Add to beginning of array (newest first)
    toasts = [toast, ...toasts];

    // Limit to 5 toasts max
    if (toasts.length > 5) {
      const removed = toasts.slice(5);
      toasts = toasts.slice(0, 5);
      removed.forEach((t) => {
        const timeout = timeoutMap.get(t.id);
        if (timeout) {
          clearTimeout(timeout);
          timeoutMap.delete(t.id);
        }
      });
    }

    // Schedule auto-dismiss
    scheduleRemoval(id, toast.duration);

    return id;
  },

  remove(id: string): void {
    const timeout = timeoutMap.get(id);
    if (timeout) {
      clearTimeout(timeout);
      timeoutMap.delete(id);
    }
    toasts = toasts.filter((t) => t.id !== id);
  },

  clear(): void {
    // Clear all timeouts
    timeoutMap.forEach((timeout) => clearTimeout(timeout));
    timeoutMap.clear();
    toasts = [];
  },

  success(title: string, body: string = '', link?: string): string {
    return this.add(title, body, link, { type: 'success' });
  },

  error(title: string, body: string = '', link?: string): string {
    return this.add(title, body, link, { type: 'error', duration: 8000 });
  },

  warning(title: string, body: string = '', link?: string): string {
    return this.add(title, body, link, { type: 'warning', duration: 6000 });
  },

  info(title: string, body: string = '', link?: string): string {
    return this.add(title, body, link, { type: 'info' });
  },
};
