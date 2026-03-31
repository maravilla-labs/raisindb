<script lang="ts">
	import { ChevronUp, ChevronDown, Trash2, GripVertical } from 'lucide-svelte';
	import type { TranscriptBlock as BlockType } from '$lib/stores/orders';
	import { blockStyles } from '$lib/config/transcriptTemplates';

	interface Props {
		block: BlockType;
		readonly?: boolean;
		canMoveUp?: boolean;
		canMoveDown?: boolean;
		onUpdate?: (content: string) => void;
		onDelete?: () => void;
		onMoveUp?: () => void;
		onMoveDown?: () => void;
	}

	let {
		block,
		readonly = false,
		canMoveUp = false,
		canMoveDown = false,
		onUpdate,
		onDelete,
		onMoveUp,
		onMoveDown
	}: Props = $props();

	let textareaRef: HTMLTextAreaElement | null = $state(null);

	const style = $derived(blockStyles[block.type] || blockStyles.notes);

	function handleInput(e: Event) {
		const target = e.target as HTMLTextAreaElement;
		onUpdate?.(target.value);
		autoResize();
	}

	function autoResize() {
		if (textareaRef) {
			textareaRef.style.height = 'auto';
			textareaRef.style.height = textareaRef.scrollHeight + 'px';
		}
	}

	$effect(() => {
		// Auto-resize on mount and when content changes
		if (textareaRef && block.content) {
			autoResize();
		}
	});
</script>

<div class="rounded-lg border-2 {style.border} {style.bg} overflow-hidden">
	<!-- Block Header -->
	<div class="flex items-center justify-between px-3 py-2 {style.headerBg}">
		<div class="flex items-center gap-2">
			{#if !readonly}
				<GripVertical class="h-4 w-4 cursor-grab text-gray-400" />
			{/if}
			<span class="text-sm font-semibold text-gray-700">{block.label}</span>
			<span class="rounded bg-white/50 px-1.5 py-0.5 text-xs text-gray-500">{block.type}</span>
		</div>

		{#if !readonly}
			<div class="flex items-center gap-1">
				<button
					type="button"
					onclick={onMoveUp}
					disabled={!canMoveUp}
					class="rounded p-1 text-gray-400 transition-colors hover:bg-white/50 hover:text-gray-600 disabled:cursor-not-allowed disabled:opacity-30"
					title="Nach oben"
				>
					<ChevronUp class="h-4 w-4" />
				</button>
				<button
					type="button"
					onclick={onMoveDown}
					disabled={!canMoveDown}
					class="rounded p-1 text-gray-400 transition-colors hover:bg-white/50 hover:text-gray-600 disabled:cursor-not-allowed disabled:opacity-30"
					title="Nach unten"
				>
					<ChevronDown class="h-4 w-4" />
				</button>
				<button
					type="button"
					onclick={onDelete}
					class="rounded p-1 text-gray-400 transition-colors hover:bg-red-100 hover:text-red-600"
					title="Löschen"
				>
					<Trash2 class="h-4 w-4" />
				</button>
			</div>
		{/if}
	</div>

	<!-- Block Content -->
	<div class="p-3">
		{#if readonly}
			<div class="whitespace-pre-wrap text-sm text-gray-700">
				{block.content || '(Kein Inhalt)'}
			</div>
		{:else}
			<textarea
				bind:this={textareaRef}
				value={block.content}
				oninput={handleInput}
				placeholder="Inhalt eingeben..."
				rows={3}
				class="w-full resize-none rounded border-0 bg-transparent p-0 text-sm text-gray-700 placeholder-gray-400 focus:outline-none focus:ring-0"
			></textarea>
		{/if}
	</div>
</div>
