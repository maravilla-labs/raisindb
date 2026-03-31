<script lang="ts">
	import type { Component } from 'svelte';
	import type { RaisinReference, TagNode } from '$lib/types';
	import { getTagNameFromReference } from '$lib/types';
	import { X } from 'lucide-svelte';
	import * as icons from 'lucide-svelte';

	interface Props {
		tag: RaisinReference;
		tagData?: TagNode | null;
		href?: string | null;
		removable?: boolean;
		onremove?: () => void;
		size?: 'sm' | 'md';
	}

	let {
		tag,
		tagData = null,
		href = null,
		removable = false,
		onremove,
		size = 'sm'
	}: Props = $props();

	// Get display label from tagData or fallback to path name
	const label = $derived(tagData?.properties?.label ?? getTagNameFromReference(tag));

	// Get icon component if specified
	const iconName = $derived(tagData?.properties?.icon);
	const IconComponent = $derived.by((): Component<{ size?: number }> | null => {
		if (!iconName) return null;
		// Convert icon name to PascalCase for lucide-svelte
		const pascalName = iconName
			?.split('-')
			?.map((part: string) => part.charAt(0).toUpperCase() + part.slice(1))
			?.join('');
		const icon = (icons as unknown as Record<string, Component<{ size?: number }>>)[pascalName];
		return icon ?? null;
	});

	// Get color or fallback to gray
	const color = $derived(tagData?.properties?.color ?? '#6B7280');

	// Size classes
	const sizeClasses = $derived(
		size === 'sm'
			? 'px-2 py-0.5 text-xs gap-1'
			: 'px-2.5 py-1 text-sm gap-1.5'
	);

	const iconSize = $derived(size === 'sm' ? 12 : 14);
</script>

{#if href}
	<a
		{href}
		class="inline-flex items-center rounded-full font-medium transition-colors hover:opacity-80 {sizeClasses}"
		style="background-color: {color}20; color: {color}; border: 1px solid {color}40;"
	>
		{#if IconComponent}
			<IconComponent size={iconSize} />
		{/if}
		<span>{label}</span>
	</a>
{:else}
	<span
		class="inline-flex items-center rounded-full font-medium {sizeClasses}"
		style="background-color: {color}20; color: {color}; border: 1px solid {color}40;"
	>
		{#if IconComponent}
			<IconComponent size={iconSize} />
		{/if}
		<span>{label}</span>
		{#if removable}
			<button
				type="button"
				onclick={onremove}
				class="ml-0.5 rounded-full p-0.5 transition-colors hover:bg-black/10"
				aria-label="Remove tag"
			>
				<X size={iconSize} />
			</button>
		{/if}
	</span>
{/if}
