<script lang="ts">
	import { Upload } from 'lucide-svelte';

	interface Props {
		onfiles?: (files: File[]) => void;
	}

	let { onfiles }: Props = $props();

	let isDragging = $state(false);
	let inputElement: HTMLInputElement;

	function handleDragEnter(e: DragEvent) {
		e.preventDefault();
		isDragging = true;
	}

	function handleDragLeave(e: DragEvent) {
		e.preventDefault();
		isDragging = false;
	}

	function handleDragOver(e: DragEvent) {
		e.preventDefault();
	}

	function handleDrop(e: DragEvent) {
		e.preventDefault();
		isDragging = false;

		const files = e.dataTransfer?.files;
		if (files && files.length > 0) {
			const validFiles = Array.from(files).filter(
				(file) => file.type.startsWith('audio/') || file.type.startsWith('image/')
			);
			if (validFiles.length > 0 && onfiles) {
				onfiles(validFiles);
			}
		}
	}

	function handleClick() {
		inputElement?.click();
	}

	function handleInputChange(e: Event) {
		const target = e.target as HTMLInputElement;
		const files = target.files;
		if (files && files.length > 0 && onfiles) {
			onfiles(Array.from(files));
		}
		target.value = '';
	}
</script>

<button
	type="button"
	class="flex w-full cursor-pointer flex-col items-center justify-center rounded-lg border-2 border-dashed p-8 transition-colors
		{isDragging
		? 'border-blue-600 bg-blue-50'
		: 'border-blue-400 bg-white hover:border-blue-500 hover:bg-blue-50'}"
	ondragenter={handleDragEnter}
	ondragleave={handleDragLeave}
	ondragover={handleDragOver}
	ondrop={handleDrop}
	onclick={handleClick}
>
	<Upload class="mb-3 h-10 w-10 text-blue-400" />
	<p class="text-center text-sm text-gray-600">
		Drag & Drop your file here or click to upload
	</p>
	<p class="mt-1 text-xs text-gray-400">Accepts audio and image files</p>
</button>

<input
	bind:this={inputElement}
	type="file"
	accept="audio/*,image/*"
	multiple
	class="hidden"
	onchange={handleInputChange}
/>
