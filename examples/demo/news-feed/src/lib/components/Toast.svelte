<script lang="ts">
	import { fly } from 'svelte/transition';
	import { X, CheckCircle, AlertCircle, Info, AlertTriangle } from 'lucide-svelte';
	import { toasts, type Toast } from '$lib/stores/toast';

	const icons = {
		success: CheckCircle,
		error: AlertCircle,
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
		success: 'text-green-400',
		error: 'text-red-400',
		info: 'text-blue-400',
		warning: 'text-yellow-400'
	};
</script>

<div class="pointer-events-none fixed right-4 top-4 z-50 flex flex-col gap-2">
	{#each $toasts as toast (toast.id)}
		<div
			in:fly={{ x: 100, duration: 300 }}
			out:fly={{ x: 100, duration: 200 }}
			class="pointer-events-auto flex w-80 items-start gap-3 rounded-lg border p-4 shadow-lg {colors[
				toast.type
			]}"
		>
			<svelte:component this={icons[toast.type]} class="h-5 w-5 shrink-0 {iconColors[toast.type]}" />
			<p class="flex-1 text-sm">{toast.message}</p>
			<button
				onclick={() => toasts.dismiss(toast.id)}
				class="shrink-0 rounded p-0.5 transition-colors hover:bg-black/5"
			>
				<X class="h-4 w-4" />
			</button>
		</div>
	{/each}
</div>
