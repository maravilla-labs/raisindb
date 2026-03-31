<script lang="ts">
	import type { ComponentType, Snippet } from 'svelte';
	import { Inbox } from 'lucide-svelte';

	interface Props {
		icon?: ComponentType;
		title: string;
		description?: string;
		children?: Snippet;
	}

	let { icon: Icon = Inbox, title, description, children }: Props = $props();
</script>

<div class="flex flex-col items-center justify-center py-12 text-center">
	<div class="mb-4 flex h-16 w-16 items-center justify-center rounded-full bg-gray-100">
		<Icon class="h-8 w-8 text-gray-400" />
	</div>
	<h3 class="text-lg font-medium text-gray-900">{title}</h3>
	{#if description}
		<p class="mt-1 text-sm text-gray-500">{description}</p>
	{/if}
	{#if children}
		<div class="mt-4">
			{@render children()}
		</div>
	{/if}
</div>
