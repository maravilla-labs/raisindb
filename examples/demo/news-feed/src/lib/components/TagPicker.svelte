<script lang="ts">
	import type { Component } from 'svelte';
	import type { RaisinReference, TagNode } from '$lib/types';
	import { tagToReference } from '$lib/types';
	import TagChip from './TagChip.svelte';
	import { Plus, Search } from 'lucide-svelte';
	import * as icons from 'lucide-svelte';

	interface Props {
		selectedTags: RaisinReference[];
		availableTags: TagNode[];
		onchange?: (tags: RaisinReference[]) => void;
		oncreate?: () => void;
		placeholder?: string;
	}

	let {
		selectedTags = $bindable([]),
		availableTags,
		onchange,
		oncreate,
		placeholder = 'Search tags...'
	}: Props = $props();

	let searchQuery = $state('');
	let isOpen = $state(false);
	let inputElement: HTMLInputElement;

	// Build a map of tag data by path for quick lookup
	const tagMap = $derived.by(() => {
		const map = new Map<string, TagNode>();
		function addTag(tag: TagNode) {
			map.set(tag.path, tag);
			tag.children?.forEach(addTag);
		}
		availableTags.forEach(addTag);
		return map;
	});

	// Flatten tags for search
	const flatTags = $derived.by(() => {
		const result: TagNode[] = [];
		function addTag(tag: TagNode, depth = 0) {
			result.push({ ...tag, children: undefined });
			tag.children?.forEach((child) => addTag(child, depth + 1));
		}
		availableTags.forEach((tag) => addTag(tag));
		return result;
	});

	// Filter tags based on search query and exclude already selected
	const filteredTags = $derived.by(() => {
		const selectedPaths = new Set(selectedTags.map((t) => t['raisin:path']));
		let filtered = flatTags.filter((tag) => !selectedPaths.has(tag.path));

		if (searchQuery.trim()) {
			const query = searchQuery.toLowerCase();
			filtered = filtered.filter(
				(tag) =>
					tag.name.toLowerCase().includes(query) ||
					tag.properties.label.toLowerCase().includes(query) ||
					tag.path.toLowerCase().includes(query)
			);
		}

		return filtered.slice(0, 10);
	});

	// Get tag data for a reference
	function getTagData(ref: RaisinReference): TagNode | null {
		return tagMap.get(ref['raisin:path']) ?? null;
	}

	type IconProps = { size?: number; style?: string; class?: string };

	// Get icon component
	function getIconComponent(iconName: string | undefined): Component<IconProps> | null {
		if (!iconName) return null;
		const pascalName = iconName
			.split('-')
			.map((part: string) => part.charAt(0).toUpperCase() + part.slice(1))
			.join('');
		const icon = (icons as unknown as Record<string, Component<IconProps>>)[pascalName];
		return icon ?? null;
	}

	function addTag(tag: TagNode) {
		const ref = tagToReference(tag);
		selectedTags = [...selectedTags, ref];
		onchange?.(selectedTags);
		searchQuery = '';
		isOpen = false;
	}

	function removeTag(index: number) {
		selectedTags = selectedTags.filter((_, i) => i !== index);
		onchange?.(selectedTags);
	}

	function handleInputFocus() {
		isOpen = true;
	}

	function handleInputBlur(e: FocusEvent) {
		// Delay closing to allow click on dropdown items
		setTimeout(() => {
			if (!e.relatedTarget || !(e.relatedTarget as HTMLElement).closest('.tag-dropdown')) {
				isOpen = false;
			}
		}, 150);
	}

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Escape') {
			isOpen = false;
			inputElement?.blur();
		}
	}
</script>

<div class="relative">
	<!-- Selected Tags -->
	{#if selectedTags.length > 0}
		<div class="mb-2 flex flex-wrap gap-1.5">
			{#each selectedTags as tag, index}
				<TagChip
					{tag}
					tagData={getTagData(tag)}
					removable
					onremove={() => removeTag(index)}
				/>
			{/each}
		</div>
	{/if}

	<!-- Search Input -->
	<div class="relative">
		<Search size={16} class="absolute left-3 top-1/2 -translate-y-1/2 text-gray-400" />
		<input
			bind:this={inputElement}
			type="text"
			bind:value={searchQuery}
			onfocus={handleInputFocus}
			onblur={handleInputBlur}
			onkeydown={handleKeydown}
			{placeholder}
			class="w-full rounded-lg border border-gray-300 py-2 pl-9 pr-3 text-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
		/>
	</div>

	<!-- Dropdown -->
	{#if isOpen}
		<div
			class="tag-dropdown absolute z-10 mt-1 max-h-64 w-full overflow-auto rounded-lg border border-gray-200 bg-white shadow-lg"
		>
			{#if filteredTags.length > 0}
				{#each filteredTags as tag}
					{@const IconComponent = getIconComponent(tag.properties.icon)}
					<button
						type="button"
						onclick={() => addTag(tag)}
						class="flex w-full items-center gap-2 px-3 py-2 text-left text-sm hover:bg-gray-50"
					>
						{#if IconComponent}
							<IconComponent
								size={16}
								style="color: {tag.properties.color ?? '#6B7280'}"
							/>
						{:else}
							<span
								class="h-4 w-4 rounded-full"
								style="background-color: {tag.properties.color ?? '#6B7280'}"
							></span>
						{/if}
						<span class="flex-1">{tag.properties.label}</span>
						<span class="text-xs text-gray-400">{tag.path.split('/').slice(-2, -1)[0]}</span>
					</button>
				{/each}
			{:else if searchQuery.trim()}
				<div class="px-3 py-4 text-center text-sm text-gray-500">
					No tags found matching "{searchQuery}"
				</div>
			{:else}
				<div class="px-3 py-4 text-center text-sm text-gray-500">
					Start typing to search tags
				</div>
			{/if}

			{#if oncreate}
				<div class="border-t border-gray-100">
					<button
						type="button"
						onclick={oncreate}
						class="flex w-full items-center gap-2 px-3 py-2 text-left text-sm text-blue-600 hover:bg-blue-50"
					>
						<Plus size={16} />
						<span>Create new tag</span>
					</button>
				</div>
			{/if}
		</div>
	{/if}
</div>
