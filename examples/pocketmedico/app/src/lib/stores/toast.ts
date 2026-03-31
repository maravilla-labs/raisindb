import { writable } from 'svelte/store';

export interface Toast {
	id: string;
	type: 'success' | 'error' | 'info' | 'warning';
	message: string;
}

function createToastStore() {
	const { subscribe, update } = writable<Toast[]>([]);

	function show(type: Toast['type'], message: string, duration = 5000): string {
		const id = crypto.randomUUID();
		update((toasts) => [...toasts, { id, type, message }]);

		if (duration > 0) {
			setTimeout(() => {
				dismiss(id);
			}, duration);
		}

		return id;
	}

	function dismiss(id: string): void {
		update((toasts) => toasts.filter((t) => t.id !== id));
	}

	return {
		subscribe,
		show,
		success: (message: string, duration?: number) => show('success', message, duration),
		error: (message: string, duration?: number) => show('error', message, duration),
		info: (message: string, duration?: number) => show('info', message, duration),
		warning: (message: string, duration?: number) => show('warning', message, duration),
		dismiss
	};
}

export const toasts = createToastStore();
