<script lang="ts">
	import type { Article, ArticleConnection, ArticleRelationType } from '$lib/types';
	import { RELATION_TYPE_META, pathToUrl } from '$lib/types';
	import { X, Search, Link, ArrowRight, RefreshCw, Pencil, XCircle, FileCheck, Bookmark } from 'lucide-svelte';

	interface Props {
		isOpen: boolean;
		availableArticles: Article[];
		editingConnection?: ArticleConnection | null;
		currentPath: string;
		onclose?: () => void;
		onsave?: (connection: ArticleConnection) => void;
	}

	let {
		isOpen = $bindable(false),
		availableArticles = [],
		editingConnection = null,
		currentPath,
		onclose,
		onsave
	}: Props = $props();

	// Form state
	let searchQuery = $state('');
	let selectedArticle = $state<Article | null>(null);
	let relationType = $state<ArticleRelationType>('similar-to');
	let weight = $state(75);
	let editorialNote = $state('');
	let showDropdown = $state(false);

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

	// Reset form when modal opens
	$effect(() => {
		if (isOpen) {
			if (editingConnection) {
				// Find the article from available articles
				selectedArticle = availableArticles.find(a => a.path === editingConnection.targetPath) || null;
				relationType = editingConnection.relationType;
				weight = editingConnection.weight;
				editorialNote = editingConnection.editorialNote || '';
			} else {
				selectedArticle = null;
				relationType = 'similar-to';
				weight = 75;
				editorialNote = '';
			}
			searchQuery = '';
			showDropdown = false;
		}
	});

	const isEditing = $derived(!!editingConnection);

	// Filter articles based on search query
	const filteredArticles = $derived(() => {
		if (!searchQuery.trim()) return availableArticles.slice(0, 10);
		const query = searchQuery.toLowerCase();
		return availableArticles
			.filter(a =>
				a.properties.title.toLowerCase().includes(query) ||
				a.path.toLowerCase().includes(query)
			)
			.slice(0, 10);
	});

	// Get all relation types for dropdown
	const relationTypes = Object.entries(RELATION_TYPE_META) as [ArticleRelationType, typeof RELATION_TYPE_META[ArticleRelationType]][];

	function handleSubmit(e: Event) {
		e.preventDefault();
		if (!selectedArticle) return;

		onsave?.({
			targetPath: selectedArticle.path,
			targetId: selectedArticle.id,
			targetTitle: selectedArticle.properties.title,
			relationType,
			weight,
			editorialNote: editorialNote.trim() || undefined
		});

		handleClose();
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

	function selectArticle(article: Article) {
		selectedArticle = article;
		searchQuery = '';
		showDropdown = false;
	}

	function handleSearchFocus() {
		showDropdown = true;
	}

	function handleSearchBlur() {
		// Delay to allow click on dropdown item
		setTimeout(() => {
			showDropdown = false;
		}, 200);
	}
</script>

{#if isOpen}
	<!-- svelte-ignore a11y_click_events_have_key_events -->
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div
		class="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
		onclick={handleBackdropClick}
	>
		<div class="w-full max-w-lg rounded-xl bg-white shadow-2xl">
			<!-- Header -->
			<div class="flex items-center justify-between border-b border-gray-200 px-6 py-4">
				<h2 class="text-lg font-semibold text-gray-900">
					{isEditing ? 'Edit Connection' : 'Add Connection'}
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
				<!-- Target Article Selection -->
				<div class="mb-5">
					<label for="target-article" class="mb-1.5 block text-sm font-medium text-gray-700">
						Target Article <span class="text-red-500">*</span>
					</label>

					{#if selectedArticle}
						<!-- Selected article display -->
						<div class="flex items-center justify-between rounded-lg border border-gray-300 bg-gray-50 px-3 py-2">
							<div class="min-w-0 flex-1">
								<p class="truncate font-medium text-gray-900">{selectedArticle.properties.title}</p>
								<p class="truncate text-xs text-gray-500">{pathToUrl(selectedArticle.path)}</p>
							</div>
							<button
								type="button"
								onclick={() => selectedArticle = null}
								class="ml-2 rounded p-1 text-gray-400 hover:bg-gray-200 hover:text-gray-600"
							>
								<X size={16} />
							</button>
						</div>
					{:else}
						<!-- Search input -->
						<div class="relative">
							<div class="pointer-events-none absolute inset-y-0 left-0 flex items-center pl-3">
								<Search size={16} class="text-gray-400" />
							</div>
							<input
								id="target-article"
								type="text"
								bind:value={searchQuery}
								onfocus={handleSearchFocus}
								onblur={handleSearchBlur}
								placeholder="Search articles..."
								class="w-full rounded-lg border border-gray-300 py-2 pl-9 pr-3 text-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
							/>

							<!-- Dropdown -->
							{#if showDropdown}
								<div class="absolute z-10 mt-1 max-h-60 w-full overflow-auto rounded-lg border border-gray-200 bg-white shadow-lg">
									{#each filteredArticles() as article}
										<button
											type="button"
											class="w-full px-3 py-2 text-left hover:bg-gray-50"
											onclick={() => selectArticle(article)}
										>
											<p class="truncate text-sm font-medium text-gray-900">{article.properties.title}</p>
											<p class="truncate text-xs text-gray-500">{pathToUrl(article.path)}</p>
										</button>
									{:else}
										<p class="px-3 py-2 text-sm text-gray-500">No articles found</p>
									{/each}
								</div>
							{/if}
						</div>
					{/if}
				</div>

				<!-- Relation Type -->
				<div class="mb-5">
					<label for="relation-type" class="mb-1.5 block text-sm font-medium text-gray-700">
						Relationship Type <span class="text-red-500">*</span>
					</label>
					<select
						id="relation-type"
						bind:value={relationType}
						class="w-full rounded-lg border border-gray-300 px-3 py-2 text-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
					>
						{#each relationTypes as [type, meta]}
							<option value={type}>{meta.label}</option>
						{/each}
					</select>
					<p class="mt-1 text-xs text-gray-500">
						{RELATION_TYPE_META[relationType].description}
					</p>
				</div>

				<!-- Weight Slider -->
				<div class="mb-5">
					<label for="weight" class="mb-1.5 block text-sm font-medium text-gray-700">
						Relevance Strength: <span class="font-semibold">{weight}%</span>
					</label>
					<input
						id="weight"
						type="range"
						min="0"
						max="100"
						step="5"
						bind:value={weight}
						class="h-2 w-full cursor-pointer appearance-none rounded-lg bg-gray-200 accent-blue-600"
					/>
					<div class="mt-1 flex justify-between text-xs text-gray-500">
						<span>Low relevance</span>
						<span>High relevance</span>
					</div>
				</div>

				<!-- Editorial Note -->
				<div class="mb-6">
					<label for="editorial-note" class="mb-1.5 block text-sm font-medium text-gray-700">
						Editorial Note <span class="text-gray-400">(optional)</span>
					</label>
					<textarea
						id="editorial-note"
						bind:value={editorialNote}
						rows={2}
						placeholder="Why are you making this connection? This helps other editors understand the relationship."
						class="w-full rounded-lg border border-gray-300 px-3 py-2 text-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
					></textarea>
				</div>

				<!-- Preview -->
				{#if selectedArticle}
					{@const PreviewIcon = iconMap[RELATION_TYPE_META[relationType].icon as keyof typeof iconMap]}
					<div class="mb-6 rounded-lg border border-gray-200 bg-gray-50 p-3">
						<p class="mb-2 text-xs font-medium uppercase tracking-wide text-gray-500">Connection Preview</p>
						<div class="flex items-center gap-2 text-sm">
							<span class="truncate text-gray-600">This article</span>
							<span
								class="inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-xs font-medium text-white"
								style="background-color: {RELATION_TYPE_META[relationType].color}"
							>
								{#if PreviewIcon}
									<PreviewIcon size={12} />
								{/if}
								{RELATION_TYPE_META[relationType].label}
							</span>
							<span class="truncate font-medium text-gray-900">{selectedArticle.properties.title}</span>
						</div>
					</div>
				{/if}

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
						disabled={!selectedArticle}
						class="rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700 disabled:cursor-not-allowed disabled:opacity-50"
					>
						{isEditing ? 'Save Changes' : 'Add Connection'}
					</button>
				</div>
			</form>
		</div>
	</div>
{/if}
