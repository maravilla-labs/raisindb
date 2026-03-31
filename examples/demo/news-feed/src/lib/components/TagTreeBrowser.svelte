<script lang="ts">
	import type { Component } from 'svelte';
	import type { TagNode } from '$lib/types';
	import { ChevronRight, ChevronDown, Plus, Pencil, Trash2 } from 'lucide-svelte';
	import * as icons from 'lucide-svelte';

	interface Props {
		tags: TagNode[];
		selectedPath?: string | null;
		onselect?: (tag: TagNode) => void;
		onedit?: (tag: TagNode) => void;
		ondelete?: (tag: TagNode) => void;
		oncreate?: (parentTag: TagNode | null) => void;
		editable?: boolean;
		isRoot?: boolean;
	}

	let {
		tags,
		selectedPath = null,
		onselect,
		onedit,
		ondelete,
		oncreate,
		editable = false,
		isRoot = true
	}: Props = $props();

	let expandedPaths = $state(new Set<string>());

	function toggleExpand(path: string) {
		const newSet = new Set(expandedPaths);
		if (newSet.has(path)) {
			newSet.delete(path);
		} else {
			newSet.add(path);
		}
		expandedPaths = newSet;
	}

	type IconProps = { size?: number; style?: string; class?: string };

	function getIconComponent(iconName: string | undefined): Component<IconProps> | null {
		if (!iconName) return null;
		const pascalName = iconName
			.split('-')
			.map((part: string) => part.charAt(0).toUpperCase() + part.slice(1))
			.join('');
		const icon = (icons as unknown as Record<string, Component<IconProps>>)[pascalName];
		return icon ?? null;
	}
</script>

<div class="space-y-1">
	{#if oncreate && editable && isRoot}
		<button
			type="button"
			onclick={() => oncreate(null)}
			class="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-sm text-blue-600 hover:bg-blue-50"
		>
			<Plus size={16} />
			<span>Add top-level tag</span>
		</button>
	{/if}

	{#each tags as tag}
		{@const IconComponent = getIconComponent(tag.properties.icon)}
		{@const hasChildren = tag.children && tag.children.length > 0}
		{@const isExpanded = expandedPaths.has(tag.path)}
		{@const isSelected = selectedPath === tag.path}

		<div>
			<div
				class="group flex items-center gap-1 rounded-lg px-2 py-1.5 transition-colors {isSelected
					? 'bg-blue-50'
					: 'hover:bg-gray-50'}"
			>
				<!-- Expand/collapse button -->
				<button
					type="button"
					onclick={() => toggleExpand(tag.path)}
					class="flex h-6 w-6 shrink-0 items-center justify-center rounded hover:bg-gray-200 {hasChildren
						? 'visible'
						: 'invisible'}"
				>
					{#if isExpanded}
						<ChevronDown size={14} class="text-gray-500" />
					{:else}
						<ChevronRight size={14} class="text-gray-500" />
					{/if}
				</button>

				<!-- Tag content -->
				<button
					type="button"
					onclick={() => onselect?.(tag)}
					class="flex flex-1 items-center gap-2 text-left"
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
					<span class="text-sm font-medium {isSelected ? 'text-blue-700' : 'text-gray-700'}">
						{tag.properties.label}
					</span>
				</button>

				<!-- Action buttons -->
				{#if editable}
					<div class="flex items-center gap-1 opacity-0 transition-opacity group-hover:opacity-100">
						{#if oncreate}
							<button
								type="button"
								onclick={() => oncreate(tag)}
								class="rounded p-1 text-gray-400 hover:bg-gray-200 hover:text-gray-600"
								title="Add child tag"
							>
								<Plus size={14} />
							</button>
						{/if}
						{#if onedit}
							<button
								type="button"
								onclick={() => onedit(tag)}
								class="rounded p-1 text-gray-400 hover:bg-gray-200 hover:text-gray-600"
								title="Edit tag"
							>
								<Pencil size={14} />
							</button>
						{/if}
						{#if ondelete}
							<button
								type="button"
								onclick={() => ondelete(tag)}
								class="rounded p-1 text-gray-400 hover:bg-red-100 hover:text-red-600"
								title="Delete tag"
							>
								<Trash2 size={14} />
							</button>
						{/if}
					</div>
				{/if}
			</div>

			<!-- Children -->
			{#if hasChildren && isExpanded}
				<div class="ml-4 border-l border-gray-200 pl-2">
					<svelte:self
						tags={tag.children ?? []}
						{selectedPath}
						{onselect}
						{onedit}
						{ondelete}
						{oncreate}
						{editable}
						isRoot={false}
					/>
				</div>
			{/if}
		</div>
	{/each}
</div>
