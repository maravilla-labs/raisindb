<script lang="ts">
	import type { TagNode, TagProperties } from '$lib/types';
	import { TAGS_PATH } from '$lib/types';
	import { X } from 'lucide-svelte';
	import IconPicker from './IconPicker.svelte';

	interface Props {
		isOpen: boolean;
		parentTag?: TagNode | null;
		editTag?: TagNode | null;
		onclose?: () => void;
		onsave?: (data: { name: string; parentPath: string; properties: TagProperties }) => void;
	}

	let {
		isOpen = $bindable(false),
		parentTag = null,
		editTag = null,
		onclose,
		onsave
	}: Props = $props();

	let name = $state('');
	let label = $state('');
	let icon = $state('');
	let color = $state('#6B7280');

	// Preset colors for quick selection
	const presetColors = [
		'#EF4444', // red
		'#F97316', // orange
		'#F59E0B', // amber
		'#EAB308', // yellow
		'#84CC16', // lime
		'#22C55E', // green
		'#10B981', // emerald
		'#14B8A6', // teal
		'#06B6D4', // cyan
		'#0EA5E9', // sky
		'#3B82F6', // blue
		'#6366F1', // indigo
		'#8B5CF6', // violet
		'#A855F7', // purple
		'#D946EF', // fuchsia
		'#EC4899', // pink
		'#F43F5E', // rose
		'#6B7280' // gray
	];

	// Reset form when modal opens
	$effect(() => {
		if (isOpen) {
			if (editTag) {
				name = editTag.name;
				label = editTag.properties.label;
				icon = editTag.properties.icon ?? '';
				color = editTag.properties.color ?? '#6B7280';
			} else {
				name = '';
				label = '';
				icon = '';
				color = '#6B7280';
			}
		}
	});

	// Auto-generate name from label
	$effect(() => {
		if (!editTag && label && !name) {
			name = label
				.toLowerCase()
				.replace(/[^a-z0-9]+/g, '-')
				.replace(/^-|-$/g, '');
		}
	});

	const parentPath = $derived(parentTag?.path ?? TAGS_PATH);
	const isEditing = $derived(!!editTag);

	function handleSubmit(e: Event) {
		e.preventDefault();
		if (!label.trim() || !name.trim()) return;

		onsave?.({
			name: name.trim(),
			parentPath,
			properties: {
				label: label.trim(),
				icon: icon || undefined,
				color
			}
		});
	}

	function handleClose() {
		isOpen = false;
		onclose?.();
	}

	function handleBackdropClick(e: MouseEvent) {
		if (e.target === e.currentTarget) {
			handleClose();
		}
	}
</script>

{#if isOpen}
	<!-- svelte-ignore a11y_click_events_have_key_events -->
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div
		class="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
		onclick={handleBackdropClick}
	>
		<div class="w-full max-w-md rounded-xl bg-white shadow-2xl">
			<!-- Header -->
			<div class="flex items-center justify-between border-b border-gray-200 px-6 py-4">
				<h2 class="text-lg font-semibold text-gray-900">
					{isEditing ? 'Edit Tag' : 'Create Tag'}
				</h2>
				<button
					type="button"
					onclick={handleClose}
					class="rounded-lg p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600"
				>
					<X size={20} />
				</button>
			</div>

			<!-- Form -->
			<form onsubmit={handleSubmit} class="p-6">
				{#if parentTag}
					<div class="mb-4 rounded-lg bg-gray-50 px-3 py-2 text-sm text-gray-600">
						Creating under: <span class="font-medium">{parentTag.properties.label}</span>
					</div>
				{/if}

				<!-- Label -->
				<div class="mb-4">
					<label for="tag-label" class="mb-1.5 block text-sm font-medium text-gray-700">
						Display Label <span class="text-red-500">*</span>
					</label>
					<input
						id="tag-label"
						type="text"
						bind:value={label}
						required
						placeholder="e.g., Machine Learning"
						class="w-full rounded-lg border border-gray-300 px-3 py-2 text-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
					/>
				</div>

				<!-- Name (slug) -->
				<div class="mb-4">
					<label for="tag-name" class="mb-1.5 block text-sm font-medium text-gray-700">
						Name (URL slug) <span class="text-red-500">*</span>
					</label>
					<input
						id="tag-name"
						type="text"
						bind:value={name}
						required
						pattern="[a-z0-9-]+"
						placeholder="e.g., machine-learning"
						class="w-full rounded-lg border border-gray-300 px-3 py-2 font-mono text-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
					/>
					<p class="mt-1 text-xs text-gray-500">
						Path: {parentPath}/{name || '...'}
					</p>
				</div>

				<!-- Icon -->
				<div class="mb-4">
					<label class="mb-1.5 block text-sm font-medium text-gray-700">Icon</label>
					<IconPicker bind:value={icon} />
				</div>

				<!-- Color -->
				<div class="mb-6">
					<label class="mb-1.5 block text-sm font-medium text-gray-700">Color</label>
					<div class="flex items-center gap-3">
						<input
							type="color"
							bind:value={color}
							class="h-10 w-14 cursor-pointer rounded border border-gray-300"
						/>
						<input
							type="text"
							bind:value={color}
							class="w-28 rounded-lg border border-gray-300 px-3 py-2 font-mono text-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
						/>
					</div>
					<!-- Preset colors -->
					<div class="mt-2 flex flex-wrap gap-1">
						{#each presetColors as presetColor}
							<button
								type="button"
								onclick={() => (color = presetColor)}
								class="h-6 w-6 rounded-full border-2 transition-transform hover:scale-110 {color ===
								presetColor
									? 'border-gray-800'
									: 'border-transparent'}"
								style="background-color: {presetColor}"
								title={presetColor}
							></button>
						{/each}
					</div>
				</div>

				<!-- Actions -->
				<div class="flex justify-end gap-3">
					<button
						type="button"
						onclick={handleClose}
						class="rounded-lg border border-gray-300 px-4 py-2 text-sm font-medium text-gray-700 hover:bg-gray-50"
					>
						Cancel
					</button>
					<button
						type="submit"
						class="rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700"
					>
						{isEditing ? 'Save Changes' : 'Create Tag'}
					</button>
				</div>
			</form>
		</div>
	</div>
{/if}
