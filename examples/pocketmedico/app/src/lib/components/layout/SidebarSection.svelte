<script lang="ts">
	import { ChevronDown, ChevronRight } from 'lucide-svelte';
	import SidebarItem from './SidebarItem.svelte';
	import type { NavItem } from '$lib/config/customerNav';

	interface Props {
		title: string;
		items: NavItem[];
		collapsible?: boolean;
		defaultExpanded?: boolean;
		getBadgeCount?: (badge: string) => number;
	}

	let { title, items, collapsible = true, defaultExpanded = true, getBadgeCount }: Props = $props();

	let expanded = $state(defaultExpanded);

	function toggle() {
		if (collapsible) {
			expanded = !expanded;
		}
	}
</script>

<div class="space-y-1">
	<!-- Section Header -->
	<button
		onclick={toggle}
		class="flex w-full items-center gap-2 px-3 py-2 text-xs font-semibold uppercase tracking-wider text-gray-500
			{collapsible ? 'cursor-pointer hover:text-gray-700' : 'cursor-default'}"
	>
		{#if collapsible}
			{#if expanded}
				<ChevronDown class="h-3 w-3" />
			{:else}
				<ChevronRight class="h-3 w-3" />
			{/if}
		{/if}
		<span>{title}</span>
	</button>

	<!-- Items -->
	{#if expanded}
		<div class="space-y-0.5">
			{#each items as item}
				<SidebarItem
					href={item.href}
					label={item.label}
					icon={item.icon}
					badge={item.badge && getBadgeCount ? getBadgeCount(item.badge) : undefined}
				/>
			{/each}
		</div>
	{/if}
</div>
