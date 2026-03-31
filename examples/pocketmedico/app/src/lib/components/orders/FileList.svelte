<script lang="ts">
	import { FileAudio, Image, X } from 'lucide-svelte';
	import type { UploadedFile } from '$lib/stores/orders';
	import { formatFileSize } from '$lib/utils/formatters';

	interface Props {
		files: UploadedFile[];
		onremove?: (fileId: string) => void;
	}

	let { files, onremove }: Props = $props();

	const typeIcons = {
		audio: FileAudio,
		image: Image
	};
</script>

{#if files.length > 0}
	<ul class="divide-y divide-gray-100 rounded-lg border border-gray-200 bg-white">
		{#each files as file (file.id)}
			{@const Icon = typeIcons[file.type]}
			<li class="flex items-center gap-3 p-3">
				<div class="flex-shrink-0">
					<Icon class="h-5 w-5 text-gray-400" />
				</div>
				<div class="min-w-0 flex-1">
					<p class="truncate text-sm font-medium text-gray-900">{file.name}</p>
					<p class="text-xs text-gray-500">{formatFileSize(file.size)}</p>
				</div>
				{#if onremove}
					<button
						type="button"
						onclick={() => onremove?.(file.id)}
						class="flex-shrink-0 rounded p-1 text-gray-400 transition-colors hover:bg-gray-100 hover:text-gray-600"
						aria-label="Remove file"
					>
						<X class="h-4 w-4" />
					</button>
				{/if}
			</li>
		{/each}
	</ul>
{/if}
