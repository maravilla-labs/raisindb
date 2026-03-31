<script lang="ts">
	import { invalidateAll } from '$app/navigation';
	import { Plus } from 'lucide-svelte';
	import { toasts } from '$lib/stores/toast';
	import type { TagNode, TagProperties } from '$lib/types';
	import { TAGS_PATH } from '$lib/types';
	import TagTreeBrowser from '$lib/components/TagTreeBrowser.svelte';
	import TagCreateModal from '$lib/components/TagCreateModal.svelte';

	let { data, form } = $props();

	let showCreateModal = $state(false);
	let parentTagForCreate = $state<TagNode | null>(null);
	let editingTag = $state<TagNode | null>(null);
	let selectedTag = $state<TagNode | null>(null);

	function handleCreate(parentTag: TagNode | null) {
		parentTagForCreate = parentTag;
		editingTag = null;
		showCreateModal = true;
	}

	function handleEdit(tag: TagNode) {
		editingTag = tag;
		parentTagForCreate = null;
		showCreateModal = true;
	}

	async function handleDelete(tag: TagNode) {
		const hasChildren = tag.children && tag.children.length > 0;
		const message = hasChildren
			? `Are you sure you want to delete "${tag.properties.label}"? This tag has child tags that will also be deleted.`
			: `Are you sure you want to delete "${tag.properties.label}"?`;

		if (!confirm(message)) return;

		const formData = new FormData();
		formData.append('path', tag.path);

		try {
			const response = await fetch('?/delete', {
				method: 'POST',
				body: formData
			});

			if (response.ok) {
				toasts.show('success', 'Tag deleted');
				if (selectedTag?.path === tag.path) {
					selectedTag = null;
				}
				await invalidateAll();
			} else {
				toasts.show('error', 'Failed to delete tag');
			}
		} catch {
			toasts.show('error', 'Failed to delete tag');
		}
	}

	async function handleSave(saveData: { name: string; parentPath: string; properties: TagProperties }) {
		const formData = new FormData();

		if (editingTag) {
			// Update existing tag
			formData.append('path', editingTag.path);
			formData.append('label', saveData.properties.label);
			if (saveData.properties.icon) formData.append('icon', saveData.properties.icon);
			if (saveData.properties.color) formData.append('color', saveData.properties.color);

			try {
				const response = await fetch('?/update', {
					method: 'POST',
					body: formData
				});

				if (response.ok) {
					toasts.show('success', 'Tag updated');
					showCreateModal = false;
					await invalidateAll();
				} else {
					toasts.show('error', 'Failed to update tag');
				}
			} catch {
				toasts.show('error', 'Failed to update tag');
			}
		} else {
			// Create new tag
			formData.append('name', saveData.name);
			formData.append('parentPath', saveData.parentPath);
			formData.append('label', saveData.properties.label);
			if (saveData.properties.icon) formData.append('icon', saveData.properties.icon);
			if (saveData.properties.color) formData.append('color', saveData.properties.color);

			try {
				const response = await fetch('?/create', {
					method: 'POST',
					body: formData
				});

				if (response.ok) {
					toasts.show('success', 'Tag created');
					showCreateModal = false;
					await invalidateAll();
				} else {
					toasts.show('error', 'Failed to create tag');
				}
			} catch {
				toasts.show('error', 'Failed to create tag');
			}
		}
	}

	$effect(() => {
		if (form?.success) {
			showCreateModal = false;
		}
	});
</script>

<svelte:head>
	<title>Tags - Settings - News Feed</title>
</svelte:head>

<div>
	<div class="mb-6 flex items-center justify-between">
		<div>
			<h2 class="text-lg font-semibold text-gray-900">Manage Tags</h2>
			<p class="text-sm text-gray-500">Organize content with hierarchical tags</p>
		</div>
		<button
			onclick={() => handleCreate(null)}
			class="inline-flex items-center gap-2 rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700"
		>
			<Plus class="h-4 w-4" />
			Add Tag
		</button>
	</div>

	{#if form?.error}
		<div class="mb-6 rounded-lg bg-red-50 p-4 text-sm text-red-700">
			{form.error}
		</div>
	{/if}

	<div class="rounded-xl border border-gray-200 bg-white">
		{#if data.tags.length > 0}
			<div class="p-4">
				<TagTreeBrowser
					tags={data.tags}
					selectedPath={selectedTag?.path}
					onselect={(tag) => (selectedTag = tag)}
					onedit={handleEdit}
					ondelete={handleDelete}
					oncreate={handleCreate}
					editable
				/>
			</div>
		{:else}
			<div class="p-12 text-center">
				<p class="text-gray-500">No tags yet.</p>
				<button
					onclick={() => handleCreate(null)}
					class="mt-4 text-blue-600 hover:underline"
				>
					Create your first tag
				</button>
			</div>
		{/if}
	</div>

	<!-- Tag Details Panel -->
	{#if selectedTag}
		<div class="mt-6 rounded-xl border border-gray-200 bg-white p-6">
			<h3 class="text-lg font-semibold text-gray-900">{selectedTag.properties.label}</h3>
			<dl class="mt-4 space-y-3 text-sm">
				<div class="flex justify-between">
					<dt class="text-gray-500">Path</dt>
					<dd class="font-mono text-gray-900">{selectedTag.path}</dd>
				</div>
				<div class="flex justify-between">
					<dt class="text-gray-500">Name</dt>
					<dd class="text-gray-900">{selectedTag.name}</dd>
				</div>
				{#if selectedTag.properties.icon}
					<div class="flex justify-between">
						<dt class="text-gray-500">Icon</dt>
						<dd class="text-gray-900">{selectedTag.properties.icon}</dd>
					</div>
				{/if}
				{#if selectedTag.properties.color}
					<div class="flex items-center justify-between">
						<dt class="text-gray-500">Color</dt>
						<dd class="flex items-center gap-2">
							<span
								class="h-4 w-4 rounded-full"
								style="background-color: {selectedTag.properties.color}"
							></span>
							<span class="font-mono text-gray-900">{selectedTag.properties.color}</span>
						</dd>
					</div>
				{/if}
			</dl>
		</div>
	{/if}
</div>

<TagCreateModal
	bind:isOpen={showCreateModal}
	parentTag={parentTagForCreate}
	editTag={editingTag}
	onclose={() => (showCreateModal = false)}
	onsave={handleSave}
/>
