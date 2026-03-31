import { writable } from 'svelte/store';

export interface Toast {
	id: string;
	type: 'success' | 'error' | 'info' | 'warning';
	message: string;
}

function createToastStore() {
	const { subscribe, update } = writable<Toast[]>([]);

	return {
		subscribe,
		show: (type: Toast['type'], message: string, duration = 5000) => {
			const id = crypto.randomUUID();
			update((toasts) => [...toasts, { id, type, message }]);

			setTimeout(() => {
				update((toasts) => toasts.filter((t) => t.id !== id));
			}, duration);

			return id;
		},
		success: (message: string) => {
			return createToastStore().show('success', message);
		},
		error: (message: string) => {
			return createToastStore().show('error', message);
		},
		dismiss: (id: string) => {
			update((toasts) => toasts.filter((t) => t.id !== id));
		}
	};
}

export const toasts = createToastStore();
