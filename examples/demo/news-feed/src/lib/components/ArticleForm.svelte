<script lang="ts">
	import {
		type Article,
		type ArticleConnection,
		type ArticleProperties,
		type Category,
		type IncomingConnection,
		type RaisinReference,
		type TagNode
	} from '$lib/types';
	import { slugify } from '$lib/utils';
	import MarkdownPreview from './MarkdownPreview.svelte';
	import TagPicker from './TagPicker.svelte';
	import TagCreateModal from './TagCreateModal.svelte';
	import ConnectionPicker from './ConnectionPicker.svelte';
	import IncomingConnectionCard from './IncomingConnectionCard.svelte';
	import { ArrowDownLeft } from 'lucide-svelte';

	interface Props {
		article?: Partial<ArticleProperties>;
		categories?: Category[];
		availableTags?: TagNode[];
		availableArticles?: Article[];
		incomingConnections?: IncomingConnection[];
		currentPath?: string;
		initialCategory?: string;
		submitLabel?: string;
		isSubmitting?: boolean;
		authorName?: string; // Auto-set author from authenticated user
	}

	let {
		article = {},
		categories = [],
		availableTags = [],
		availableArticles = [],
		incomingConnections = [],
		currentPath = '',
		initialCategory = '',
		submitLabel = 'Create Article',
		isSubmitting = false,
		authorName
	}: Props = $props();

	let title = $state(article.title ?? '');
	let slug = $state(article.slug ?? '');
	let excerpt = $state(article.excerpt ?? '');
	let body = $state(article.body ?? '');
	let category = $state(initialCategory || (categories[0]?.slug ?? ''));
	let keywordsInput = $state(article?.keywords?.join(', ') ?? '');
	let selectedTags = $state<RaisinReference[]>(article.tags ?? []);
	let featured = $state(article.featured ?? false);
	let status = $state<'draft' | 'published'>(article.status ?? 'published');
	let publishingDate = $state(
		article.publishing_date
			? article.publishing_date.slice(0, 16)
			: new Date().toISOString().slice(0, 16)
	);
	let author = $state(article.author ?? '');
	let imageUrl = $state(article.imageUrl ?? '');
	let connections = $state<ArticleConnection[]>(article.connections ?? []);

	let showPreview = $state(false);
	let slugManuallyEdited = $state(!!article.slug);
	let showTagCreateModal = $state(false);

	function handleTitleChange() {
		if (!slugManuallyEdited) {
			slug = slugify(title);
		}
	}

	function handleSlugChange() {
		slugManuallyEdited = true;
	}
</script>

<form method="POST" class="space-y-6">
	<div class="grid gap-6 md:grid-cols-2">
		<div class="md:col-span-2">
			<label for="title" class="block text-sm font-medium text-gray-700">
				Title <span class="text-red-500">*</span>
			</label>
			<input
				type="text"
				id="title"
				name="title"
				bind:value={title}
				oninput={handleTitleChange}
				required
				class="mt-1 block w-full rounded-lg border border-gray-300 px-3 py-2 shadow-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
				placeholder="Enter article title"
			/>
		</div>

		<div>
			<label for="slug" class="block text-sm font-medium text-gray-700">
				Slug <span class="text-red-500">*</span>
			</label>
			<input
				type="text"
				id="slug"
				name="slug"
				bind:value={slug}
				oninput={handleSlugChange}
				required
				class="mt-1 block w-full rounded-lg border border-gray-300 px-3 py-2 font-mono text-sm shadow-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
				placeholder="article-slug"
			/>
			<p class="mt-1 text-xs text-gray-500">URL-friendly identifier</p>
		</div>

		<div>
			<label for="category" class="block text-sm font-medium text-gray-700">
				Category <span class="text-red-500">*</span>
			</label>
			<select
				id="category"
				name="category"
				bind:value={category}
				required
				class="mt-1 block w-full rounded-lg border border-gray-300 px-3 py-2 shadow-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
			>
				{#each categories as cat}
					<option value={cat.slug}>{cat.properties.label}</option>
				{/each}
			</select>
		</div>

		<div class="md:col-span-2">
			<label for="excerpt" class="block text-sm font-medium text-gray-700">Excerpt</label>
			<textarea
				id="excerpt"
				name="excerpt"
				bind:value={excerpt}
				rows="2"
				class="mt-1 block w-full rounded-lg border border-gray-300 px-3 py-2 shadow-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
				placeholder="Brief summary of the article"
			></textarea>
		</div>

		<div class="md:col-span-2">
			<div class="flex items-center justify-between">
				<label for="body" class="block text-sm font-medium text-gray-700">
					Content (Markdown)
				</label>
				<button
					type="button"
					onclick={() => (showPreview = !showPreview)}
					class="text-sm text-blue-600 hover:text-blue-800"
				>
					{showPreview ? 'Edit' : 'Preview'}
				</button>
			</div>
			{#if showPreview}
				<div
					class="mt-1 min-h-[300px] rounded-lg border border-gray-300 bg-gray-50 p-4"
				>
					<MarkdownPreview content={body} />
				</div>
			{:else}
				<textarea
					id="body"
					name="body"
					bind:value={body}
					rows="12"
					class="mt-1 block w-full rounded-lg border border-gray-300 px-3 py-2 font-mono text-sm shadow-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
					placeholder="Write your article content in Markdown..."
				></textarea>
			{/if}
		</div>

		<div class="md:col-span-2">
			<label class="block text-sm font-medium text-gray-700">Tags</label>
			<div class="mt-1">
				<TagPicker
					bind:selectedTags
					{availableTags}
					oncreate={() => (showTagCreateModal = true)}
					placeholder="Search and select tags..."
				/>
			</div>
			<!-- Hidden input to submit tags as JSON -->
			<input type="hidden" name="tags" value={JSON.stringify(selectedTags)} />
		</div>

		<!-- Story Connections Section -->
		{#if availableArticles.length > 0}
			<div class="md:col-span-2">
				<div class="rounded-xl border border-gray-200 bg-gradient-to-br from-gray-50 to-white p-5">
					<ConnectionPicker
						bind:connections
						{availableArticles}
						{currentPath}
					/>
				</div>
				<!-- Hidden input to submit connections as JSON -->
				<input type="hidden" name="connections" value={JSON.stringify(connections)} />
			</div>

			<!-- Incoming Connections (Read-only) -->
			{#if incomingConnections.length > 0}
				<div class="md:col-span-2">
					<div class="rounded-xl border border-amber-200 bg-gradient-to-br from-amber-50 to-white p-5">
						<div class="mb-4 flex items-center gap-2">
							<ArrowDownLeft size={20} class="text-amber-600" />
							<h3 class="text-sm font-semibold text-gray-900">Incoming Connections</h3>
							<span class="rounded-full bg-amber-100 px-2 py-0.5 text-xs font-medium text-amber-700">
								{incomingConnections.length}
							</span>
						</div>
						<p class="mb-4 text-sm text-gray-500">
							These articles have connections pointing to this article. You can view but not edit them here.
						</p>
						<div class="space-y-2">
							{#each incomingConnections as incoming}
								<IncomingConnectionCard connection={incoming} />
							{/each}
						</div>
					</div>
				</div>
			{/if}
		{/if}

		<div>
			<label for="keywords" class="block text-sm font-medium text-gray-700">Keywords</label>
			<input
				type="text"
				id="keywords"
				name="keywords"
				bind:value={keywordsInput}
				class="mt-1 block w-full rounded-lg border border-gray-300 px-3 py-2 shadow-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
				placeholder="rust, web, programming"
			/>
			<p class="mt-1 text-xs text-gray-500">Comma-separated keywords for search</p>
		</div>

		<div>
			<label for="author" class="block text-sm font-medium text-gray-700">Author</label>
			{#if authorName}
				<!-- Show read-only author for authenticated users -->
				<div
					class="mt-1 block w-full rounded-lg border border-gray-200 bg-gray-50 px-3 py-2 text-gray-700"
				>
					{authorName}
				</div>
			{:else}
				<input
					type="text"
					id="author"
					name="author"
					bind:value={author}
					class="mt-1 block w-full rounded-lg border border-gray-300 px-3 py-2 shadow-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
					placeholder="John Doe"
				/>
			{/if}
		</div>

		<div>
			<label for="publishing_date" class="block text-sm font-medium text-gray-700">
				Publishing Date
			</label>
			<input
				type="datetime-local"
				id="publishing_date"
				name="publishing_date"
				bind:value={publishingDate}
				class="mt-1 block w-full rounded-lg border border-gray-300 px-3 py-2 shadow-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
			/>
			<p class="mt-1 text-xs text-gray-500">
				When this article should be published. Future dates will hide the article until then.
			</p>
		</div>

		<div class="md:col-span-2">
			<label for="imageUrl" class="block text-sm font-medium text-gray-700">Image URL</label>
			<input
				type="url"
				id="imageUrl"
				name="imageUrl"
				bind:value={imageUrl}
				class="mt-1 block w-full rounded-lg border border-gray-300 px-3 py-2 shadow-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
				placeholder="https://example.com/image.jpg"
			/>
			{#if imageUrl}
				<div class="mt-2">
					<img
						src={imageUrl}
						alt="Preview"
						class="h-32 w-auto rounded-lg object-cover"
						onerror={(e) => ((e.target as HTMLImageElement).style.display = 'none')}
					/>
				</div>
			{/if}
		</div>

		<div class="flex items-center gap-6">
			<label class="flex items-center gap-2">
				<input
					type="checkbox"
					name="featured"
					bind:checked={featured}
					class="h-4 w-4 rounded border-gray-300 text-blue-600 focus:ring-blue-500"
				/>
				<span class="text-sm text-gray-700">Featured article</span>
			</label>
		</div>

		<div>
			<label class="block text-sm font-medium text-gray-700">Status</label>
			<div class="mt-2 flex gap-4">
				<label class="flex items-center gap-2">
					<input
						type="radio"
						name="status"
						value="draft"
						bind:group={status}
						class="h-4 w-4 border-gray-300 text-blue-600 focus:ring-blue-500"
					/>
					<span class="text-sm text-gray-700">Draft</span>
				</label>
				<label class="flex items-center gap-2">
					<input
						type="radio"
						name="status"
						value="published"
						bind:group={status}
						class="h-4 w-4 border-gray-300 text-blue-600 focus:ring-blue-500"
					/>
					<span class="text-sm text-gray-700">Published</span>
				</label>
			</div>
		</div>
	</div>

	<div class="flex items-center justify-end gap-4 border-t border-gray-200 pt-6">
		<a
			href="/"
			class="rounded-lg border border-gray-300 px-4 py-2 text-sm font-medium text-gray-700 hover:bg-gray-50"
		>
			Cancel
		</a>
		<button
			type="submit"
			disabled={isSubmitting}
			class="rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700 disabled:opacity-50"
		>
			{isSubmitting ? 'Saving...' : submitLabel}
		</button>
	</div>
</form>

<TagCreateModal
	bind:isOpen={showTagCreateModal}
	onsave={(data) => {
		// For now, just close the modal - actual tag creation requires server action
		showTagCreateModal = false;
		// TODO: Implement tag creation via server action
	}}
/>
