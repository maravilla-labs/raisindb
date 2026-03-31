<script lang="ts">
	import { invalidateAll } from '$app/navigation';
	import { GripVertical, Plus, Pencil, Trash2, X, Check } from 'lucide-svelte';
	import { toasts } from '$lib/stores/toast';
	import { slugify } from '$lib/utils';
	import type { Category } from '$lib/types';

	let { data, form }: { data: { categories: Category[] }; form: { success?: boolean; error?: string } | null } = $props();

	let showAddForm = $state(false);
	let editingCategory = $state<Category | null>(null);
	let draggedCategory = $state<Category | null>(null);
	let dragOverCategory = $state<Category | null>(null);

	// New category form state
	let newName = $state('');
	let newSlug = $state('');
	let newLabel = $state('');
	let newColor = $state('#3B82F6');
	let slugManuallyEdited = $state(false);

	// Edit form state
	let editName = $state('');
	let editLabel = $state('');
	let editColor = $state('');

	function handleNameChange() {
		if (!slugManuallyEdited) {
			newSlug = slugify(newName);
			newLabel = newName;
		}
	}

	function resetAddForm() {
		newName = '';
		newSlug = '';
		newLabel = '';
		newColor = '#3B82F6';
		slugManuallyEdited = false;
		showAddForm = false;
	}

	function startEdit(category: Category) {
		editingCategory = category;
		editName = category.name;
		editLabel = category.properties.label;
		editColor = category.properties.color;
	}

	function cancelEdit() {
		editingCategory = null;
	}

	// Drag and drop handlers
	function handleDragStart(e: DragEvent, category: Category) {
		draggedCategory = category;
		if (e.dataTransfer) {
			e.dataTransfer.effectAllowed = 'move';
			e.dataTransfer.setData('text/plain', category.path);
		}
	}

	function handleDragOver(e: DragEvent, category: Category) {
		e.preventDefault();
		if (draggedCategory && draggedCategory.path !== category.path) {
			dragOverCategory = category;
		}
	}

	function handleDragLeave() {
		dragOverCategory = null;
	}

	function handleDragEnd() {
		draggedCategory = null;
		dragOverCategory = null;
	}

	async function handleDrop(e: DragEvent, targetCategory: Category) {
		e.preventDefault();
		if (!draggedCategory || draggedCategory.path === targetCategory.path) {
			handleDragEnd();
			return;
		}

		const formData = new FormData();
		formData.append('sourcePath', draggedCategory.path);
		formData.append('targetPath', targetCategory.path);

		try {
			const response = await fetch('?/reorder', {
				method: 'POST',
				body: formData
			});

			if (response.ok) {
				toasts.show('success', 'Category order updated');
				await invalidateAll();
			} else {
				toasts.show('error', 'Failed to reorder categories');
			}
		} catch {
			toasts.show('error', 'Failed to reorder categories');
		}

		handleDragEnd();
	}

	async function handleDelete(category: Category) {
		if (!confirm(`Are you sure you want to delete "${category.properties.label}"? This will fail if the category contains articles.`)) {
			return;
		}

		const formData = new FormData();
		formData.append('path', category.path);

		try {
			const response = await fetch('?/delete', {
				method: 'POST',
				body: formData
			});

			if (response.ok) {
				toasts.show('success', 'Category deleted');
				await invalidateAll();
			} else {
				toasts.show('error', 'Failed to delete category. Make sure it has no articles.');
			}
		} catch {
			toasts.show('error', 'Failed to delete category');
		}
	}

	$effect(() => {
		if (form?.success) {
			resetAddForm();
			cancelEdit();
		}
	});
</script>

<svelte:head>
	<title>Categories - Settings - News Feed</title>
</svelte:head>

<div>
	<div class="mb-6 flex items-center justify-between">
		<div>
			<h2 class="text-lg font-semibold text-gray-900">Manage Categories</h2>
			<p class="text-sm text-gray-500">Drag and drop to reorder categories</p>
		</div>
		{#if !showAddForm}
			<button
				onclick={() => (showAddForm = true)}
				class="inline-flex items-center gap-2 rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700"
			>
				<Plus class="h-4 w-4" />
				Add Category
			</button>
		{/if}
	</div>

	{#if form?.error}
		<div class="mb-6 rounded-lg bg-red-50 p-4 text-sm text-red-700">
			{form.error}
		</div>
	{/if}

	<!-- Add Category Form -->
	{#if showAddForm}
		<form method="POST" action="?/create" class="mb-6 rounded-xl border border-gray-200 bg-white p-6 shadow-sm">
			<h2 class="mb-4 text-lg font-semibold text-gray-900">New Category</h2>
			<div class="grid gap-4 sm:grid-cols-2">
				<div>
					<label for="name" class="block text-sm font-medium text-gray-700">Name</label>
					<input
						type="text"
						id="name"
						name="name"
						bind:value={newName}
						oninput={handleNameChange}
						required
						class="mt-1 block w-full rounded-lg border border-gray-300 px-3 py-2 text-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
						placeholder="Technology"
					/>
				</div>
				<div>
					<label for="slug" class="block text-sm font-medium text-gray-700">Slug</label>
					<input
						type="text"
						id="slug"
						name="slug"
						bind:value={newSlug}
						oninput={() => (slugManuallyEdited = true)}
						required
						class="mt-1 block w-full rounded-lg border border-gray-300 px-3 py-2 font-mono text-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
						placeholder="tech"
					/>
				</div>
				<div>
					<label for="label" class="block text-sm font-medium text-gray-700">Display Label</label>
					<input
						type="text"
						id="label"
						name="label"
						bind:value={newLabel}
						required
						class="mt-1 block w-full rounded-lg border border-gray-300 px-3 py-2 text-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
						placeholder="Technology"
					/>
				</div>
				<div>
					<label for="color" class="block text-sm font-medium text-gray-700">Color</label>
					<div class="mt-1 flex items-center gap-2">
						<input
							type="color"
							id="color"
							name="color"
							bind:value={newColor}
							class="h-10 w-14 cursor-pointer rounded border border-gray-300"
						/>
						<input
							type="text"
							bind:value={newColor}
							class="block w-full rounded-lg border border-gray-300 px-3 py-2 font-mono text-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
							placeholder="#3B82F6"
						/>
					</div>
				</div>
			</div>
			<div class="mt-4 flex justify-end gap-2">
				<button
					type="button"
					onclick={resetAddForm}
					class="rounded-lg border border-gray-300 px-4 py-2 text-sm font-medium text-gray-700 hover:bg-gray-50"
				>
					Cancel
				</button>
				<button
					type="submit"
					class="rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700"
				>
					Create Category
				</button>
			</div>
		</form>
	{/if}

	<!-- Categories List -->
	<div class="space-y-2">
		{#each data.categories as category (category.id)}
			<div
				draggable="true"
				ondragstart={(e) => handleDragStart(e, category)}
				ondragover={(e) => handleDragOver(e, category)}
				ondragleave={handleDragLeave}
				ondragend={handleDragEnd}
				ondrop={(e) => handleDrop(e, category)}
				class="rounded-xl border bg-white transition-all {draggedCategory?.path === category.path
					? 'opacity-50'
					: ''} {dragOverCategory?.path === category.path
					? 'border-blue-500 ring-2 ring-blue-200'
					: 'border-gray-200'}"
			>
				{#if editingCategory?.path === category.path}
					<!-- Edit Mode -->
					<form method="POST" action="?/update" class="p-4">
						<input type="hidden" name="path" value={category.path} />
						<div class="grid gap-4 sm:grid-cols-3">
							<div>
								<label class="block text-xs font-medium text-gray-500">Name</label>
								<input
									type="text"
									name="name"
									bind:value={editName}
									required
									class="mt-1 block w-full rounded border border-gray-300 px-2 py-1.5 text-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
								/>
							</div>
							<div>
								<label class="block text-xs font-medium text-gray-500">Label</label>
								<input
									type="text"
									name="label"
									bind:value={editLabel}
									required
									class="mt-1 block w-full rounded border border-gray-300 px-2 py-1.5 text-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
								/>
							</div>
							<div>
								<label class="block text-xs font-medium text-gray-500">Color</label>
								<div class="mt-1 flex items-center gap-2">
									<input
										type="color"
										name="color"
										bind:value={editColor}
										class="h-8 w-10 cursor-pointer rounded border border-gray-300"
									/>
									<input
										type="text"
										bind:value={editColor}
										class="block w-full rounded border border-gray-300 px-2 py-1.5 font-mono text-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
									/>
								</div>
							</div>
						</div>
						<div class="mt-3 flex justify-end gap-2">
							<button
								type="button"
								onclick={cancelEdit}
								class="rounded p-1.5 text-gray-400 hover:bg-gray-100 hover:text-gray-600"
							>
								<X class="h-4 w-4" />
							</button>
							<button
								type="submit"
								class="rounded p-1.5 text-green-600 hover:bg-green-50"
							>
								<Check class="h-4 w-4" />
							</button>
						</div>
					</form>
				{:else}
					<!-- View Mode -->
					<div class="flex items-center gap-4 p-4">
						<div class="cursor-grab text-gray-400 hover:text-gray-600">
							<GripVertical class="h-5 w-5" />
						</div>
						<div
							class="h-8 w-8 rounded-lg"
							style="background-color: {category.properties.color}"
						></div>
						<div class="flex-1">
							<div class="font-medium text-gray-900">{category.properties.label}</div>
							<div class="text-sm text-gray-500">/{category.slug}</div>
						</div>
						<div class="flex items-center gap-1">
							<button
								onclick={() => startEdit(category)}
								class="rounded p-2 text-gray-400 hover:bg-gray-100 hover:text-gray-600"
								title="Edit"
							>
								<Pencil class="h-4 w-4" />
							</button>
							<button
								onclick={() => handleDelete(category)}
								class="rounded p-2 text-gray-400 hover:bg-red-50 hover:text-red-600"
								title="Delete"
							>
								<Trash2 class="h-4 w-4" />
							</button>
						</div>
					</div>
				{/if}
			</div>
		{/each}
	</div>

	{#if data.categories.length === 0}
		<div class="rounded-xl border-2 border-dashed border-gray-300 p-12 text-center">
			<p class="text-gray-500">No categories yet.</p>
			<button
				onclick={() => (showAddForm = true)}
				class="mt-4 text-blue-600 hover:underline"
			>
				Create your first category
			</button>
		</div>
	{/if}
</div>

