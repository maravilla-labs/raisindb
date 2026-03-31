/**
 * Toast notification store.
 *
 * Manages toast notifications with auto-dismiss functionality.
 */
import { writable } from 'svelte/store';

export interface Toast {
  id: string;
  type: 'message' | 'relationship_request' | 'system' | 'success' | 'error';
  title: string;
  body?: string;
  link?: string;
  duration?: number; // ms, default 4000
}

const DEFAULT_DURATION = 4000;

function createToastStore() {
  const { subscribe, update } = writable<Toast[]>([]);

  return {
    subscribe,

    /**
     * Show a toast notification.
     */
    show(toast: Omit<Toast, 'id'>) {
      const id = `toast-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
      const duration = toast.duration ?? DEFAULT_DURATION;

      update(toasts => [...toasts, { ...toast, id }]);

      // Auto-dismiss after duration
      if (duration > 0) {
        setTimeout(() => {
          this.dismiss(id);
        }, duration);
      }

      return id;
    },

    /**
     * Dismiss a specific toast.
     */
    dismiss(id: string) {
      update(toasts => toasts.filter(t => t.id !== id));
    },

    /**
     * Dismiss all toasts.
     */
    dismissAll() {
      update(() => []);
    },

    /**
     * Show a message notification toast.
     */
    message(title: string, body?: string, link?: string) {
      return this.show({ type: 'message', title, body, link });
    },

    /**
     * Show a friend request toast.
     */
    friendRequest(title: string, body?: string) {
      return this.show({ type: 'relationship_request', title, body });
    },

    /**
     * Show a success toast.
     */
    success(title: string, body?: string) {
      return this.show({ type: 'success', title, body });
    },

    /**
     * Show an error toast.
     */
    error(title: string, body?: string) {
      return this.show({ type: 'error', title, body, duration: 6000 });
    },
  };
}

export const toastStore = createToastStore();
