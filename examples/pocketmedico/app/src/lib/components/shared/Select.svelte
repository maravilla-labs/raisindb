<script lang="ts">
	import { ChevronDown } from 'lucide-svelte';
	import type { HTMLSelectAttributes } from 'svelte/elements';

	interface Option {
		value: string;
		label: string;
	}

	interface Props extends Omit<HTMLSelectAttributes, 'value'> {
		label?: string;
		error?: string;
		hint?: string;
		options: Option[];
		placeholder?: string;
		value?: string;
	}

	let {
		label,
		error,
		hint,
		options,
		placeholder,
		id,
		value = $bindable(''),
		class: className = '',
		...rest
	}: Props = $props();

	const selectId = id ?? `select-${crypto.randomUUID().slice(0, 8)}`;
</script>

<div class="space-y-1">
	{#if label}
		<label for={selectId} class="block text-sm font-medium text-gray-700">
			{label}
		</label>
	{/if}

	<div class="relative">
		<select
			id={selectId}
			bind:value
			class="block w-full appearance-none rounded-lg border bg-white px-3 py-2 pr-10 text-sm transition-colors
				focus:outline-none focus:ring-2 focus:ring-offset-0
				{error
				? 'border-red-300 focus:border-red-500 focus:ring-red-500'
				: 'border-gray-300 focus:border-blue-500 focus:ring-blue-500'}
				disabled:cursor-not-allowed disabled:bg-gray-50 disabled:text-gray-500
				{className}"
			{...rest}
		>
			{#if placeholder}
				<option value="" disabled>{placeholder}</option>
			{/if}
			{#each options as option}
				<option value={option.value}>{option.label}</option>
			{/each}
		</select>
		<ChevronDown class="pointer-events-none absolute right-3 top-1/2 h-4 w-4 -translate-y-1/2 text-gray-400" />
	</div>

	{#if error}
		<p class="text-sm text-red-600">{error}</p>
	{:else if hint}
		<p class="text-sm text-gray-500">{hint}</p>
	{/if}
</div>
