<script lang="ts">
	import type { ArticleConnection } from '$lib/types';
	import { RELATION_TYPE_META, pathToUrl } from '$lib/types';
	import { X, ChevronDown, ChevronUp, Pencil, ArrowRight, RefreshCw, XCircle, FileCheck, Link, Bookmark } from 'lucide-svelte';

	interface Props {
		connection: ArticleConnection;
		onremove?: () => void;
		onedit?: () => void;
	}

	let { connection, onremove, onedit }: Props = $props();

	let expanded = $state(false);

	const meta = $derived(RELATION_TYPE_META[connection.relationType]);

	// Get icon component for relation type
	const iconMap = {
		'arrow-right': ArrowRight,
		'refresh-cw': RefreshCw,
		'pencil': Pencil,
		'x-circle': XCircle,
		'file-check': FileCheck,
		'link': Link,
		'bookmark': Bookmark
	};

	const IconComponent = $derived(iconMap[meta.icon as keyof typeof iconMap]);
</script>

<div class="rounded-lg border border-gray-200 bg-white transition-colors hover:border-gray-300">
	<!-- Main row -->
	<div class="flex items-start gap-3 p-3">
		<!-- Relation type badge -->
		<span
			class="mt-0.5 inline-flex shrink-0 items-center gap-1 rounded-full px-2 py-0.5 text-xs font-medium text-white"
			style="background-color: {meta.color}"
		>
			{#if IconComponent}
				<IconComponent size={12} />
			{/if}
			{meta.label}
		</span>

		<!-- Target article info -->
		<div class="min-w-0 flex-1">
			<p class="truncate font-medium text-gray-900">{connection.targetTitle}</p>
			<p class="truncate text-xs text-gray-500">{pathToUrl(connection.targetPath)}</p>

			<!-- Weight bar -->
			<div class="mt-2 flex items-center gap-2">
				<div class="h-1.5 flex-1 overflow-hidden rounded-full bg-gray-200">
					<div
						class="h-full rounded-full transition-all"
						style="width: {connection.weight}%; background-color: {meta.color}"
					></div>
				</div>
				<span class="text-xs font-medium text-gray-500">{connection.weight}%</span>
			</div>
		</div>

		<!-- Actions -->
		<div class="flex shrink-0 items-center gap-1">
			{#if connection.editorialNote}
				<button
					type="button"
					onclick={() => expanded = !expanded}
					class="rounded p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600"
					title={expanded ? 'Hide note' : 'Show note'}
				>
					{#if expanded}
						<ChevronUp size={16} />
					{:else}
						<ChevronDown size={16} />
					{/if}
				</button>
			{/if}
			{#if onedit}
				<button
					type="button"
					onclick={onedit}
					class="rounded p-1 text-gray-400 hover:bg-gray-100 hover:text-blue-600"
					title="Edit connection"
				>
					<Pencil size={16} />
				</button>
			{/if}
			{#if onremove}
				<button
					type="button"
					onclick={onremove}
					class="rounded p-1 text-gray-400 hover:bg-gray-100 hover:text-red-600"
					title="Remove connection"
				>
					<X size={16} />
				</button>
			{/if}
		</div>
	</div>

	<!-- Editorial note (expandable) -->
	{#if connection.editorialNote && expanded}
		<div class="border-t border-gray-100 bg-gray-50 px-3 py-2">
			<p class="text-xs text-gray-600">
				<span class="font-medium">Editorial note:</span>
				{connection.editorialNote}
			</p>
		</div>
	{/if}
</div>
