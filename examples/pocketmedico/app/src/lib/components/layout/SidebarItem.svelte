<script lang="ts">
	import { page } from '$app/stores';
	import type { ComponentType } from 'svelte';

	interface Props {
		href: string;
		label: string;
		icon: ComponentType;
		badge?: number;
	}

	let { href, label, icon: Icon, badge }: Props = $props();

	let isActive = $derived($page.url.pathname === href || ($page.url.pathname.startsWith(href) && href !== '/dashboard'));
</script>

<a
	{href}
	class="flex items-center gap-3 rounded-lg px-3 py-2 text-sm font-medium transition-colors
		{isActive
		? 'bg-blue-500 text-white'
		: 'text-gray-700 hover:bg-gray-100'}"
>
	<Icon class="h-5 w-5 flex-shrink-0" />
	<span class="flex-1 truncate">{label}</span>
	{#if badge !== undefined && badge > 0}
		<span
			class="flex h-5 min-w-5 items-center justify-center rounded-full px-1.5 text-xs font-semibold
				{isActive ? 'bg-white text-blue-600' : 'bg-blue-100 text-blue-600'}"
		>
			{badge > 99 ? '99+' : badge}
		</span>
	{/if}
</a>
