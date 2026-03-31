<script lang="ts">
	import type { Snippet } from 'svelte';
	import type { HTMLButtonAttributes } from 'svelte/elements';

	interface Props extends HTMLButtonAttributes {
		variant?: 'primary' | 'secondary' | 'ghost' | 'danger';
		size?: 'sm' | 'md' | 'lg';
		loading?: boolean;
		children: Snippet;
	}

	let {
		variant = 'primary',
		size = 'md',
		loading = false,
		disabled,
		class: className = '',
		children,
		...rest
	}: Props = $props();

	const variants = {
		primary: 'bg-blue-600 text-white hover:bg-blue-700 focus:ring-blue-500',
		secondary: 'bg-white text-gray-700 border border-gray-300 hover:bg-gray-50 focus:ring-blue-500',
		ghost: 'bg-transparent text-gray-600 hover:bg-gray-100 focus:ring-gray-500',
		danger: 'bg-red-600 text-white hover:bg-red-700 focus:ring-red-500'
	};

	const sizes = {
		sm: 'px-3 py-1.5 text-sm',
		md: 'px-4 py-2 text-sm',
		lg: 'px-6 py-3 text-base'
	};
</script>

<button
	class="inline-flex items-center justify-center gap-2 rounded-lg font-medium transition-colors
		focus:outline-none focus:ring-2 focus:ring-offset-2
		disabled:cursor-not-allowed disabled:opacity-50
		{variants[variant]} {sizes[size]} {className}"
	disabled={disabled || loading}
	{...rest}
>
	{#if loading}
		<svg class="h-4 w-4 animate-spin" viewBox="0 0 24 24" fill="none">
			<circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4" />
			<path
				class="opacity-75"
				fill="currentColor"
				d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"
			/>
		</svg>
	{/if}
	{@render children()}
</button>
