<script lang="ts">
	import { toasts } from '$lib/stores/toast';
	import { CheckCircle, XCircle, Info, AlertTriangle, X } from 'lucide-svelte';
	import { fly } from 'svelte/transition';

	const icons = {
		success: CheckCircle,
		error: XCircle,
		info: Info,
		warning: AlertTriangle
	};

	const colors = {
		success: 'bg-green-50 text-green-800 border-green-200',
		error: 'bg-red-50 text-red-800 border-red-200',
		info: 'bg-blue-50 text-blue-800 border-blue-200',
		warning: 'bg-yellow-50 text-yellow-800 border-yellow-200'
	};

	const iconColors = {
		success: 'text-green-500',
		error: 'text-red-500',
		info: 'text-blue-500',
		warning: 'text-yellow-500'
	};
</script>

<div class="fixed bottom-4 right-4 z-50 flex flex-col gap-2">
	{#each $toasts as toast (toast.id)}
		{@const Icon = icons[toast.type]}
		<div
			transition:fly={{ x: 100, duration: 200 }}
			class="flex items-center gap-3 rounded-lg border px-4 py-3 shadow-lg {colors[toast.type]}"
		>
			<Icon class="h-5 w-5 flex-shrink-0 {iconColors[toast.type]}" />
			<span class="text-sm font-medium">{toast.message}</span>
			<button
				onclick={() => toasts.dismiss(toast.id)}
				class="ml-2 rounded-lg p-1 hover:bg-black/5"
			>
				<X class="h-4 w-4" />
			</button>
		</div>
	{/each}
</div>
