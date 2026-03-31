<script lang="ts">
	import { Search, Tag, X } from 'lucide-svelte';
	import ArticleCard from '$lib/components/ArticleCard.svelte';
	import SearchInput from '$lib/components/SearchInput.svelte';
	import TagChip from '$lib/components/TagChip.svelte';
	import type { TagNode } from '$lib/types';

	let { data } = $props();

	// Build tag map from available tags
	const tagMap = $derived.by(() => {
		const map = new Map<string, TagNode>();
		function addTag(tag: TagNode) {
			map.set(tag.path, tag);
			tag.children?.forEach(addTag);
		}
		data.tags?.forEach(addTag);
		return map;
	});

	// Get the tag data for the current filter
	const currentTagData = $derived(data.tag ? tagMap.get(data.tag) : null);

	// Create a fake reference object for display
	const currentTagRef = $derived(
		data.tag
			? {
					'raisin:ref': currentTagData?.id || '',
					'raisin:workspace': 'social',
					'raisin:path': data.tag
				}
			: null
	);
</script>

<svelte:head>
	<title>{data.tag ? `Tag: ${data.tag}` : data.query ? `Search: ${data.query}` : 'Search'} - News Feed</title>
</svelte:head>

<div class="space-y-8">
	<div class="mx-auto max-w-xl">
		<SearchInput value={data.query} placeholder="Search articles..." />
	</div>

	{#if data.tag && currentTagRef}
		<div class="flex items-center gap-3">
			<Tag class="h-5 w-5 text-gray-400" />
			<h1 class="text-xl font-bold text-gray-900">
				Articles tagged with
			</h1>
			<TagChip tag={currentTagRef} tagData={currentTagData} size="md" />
			<a
				href="/search"
				class="rounded-full p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600"
				title="Clear filter"
			>
				<X size={16} />
			</a>
			<span class="text-gray-500">({data.articles.length} found)</span>
		</div>

		{#if data.articles.length > 0}
			<div class="grid gap-6 sm:grid-cols-2 lg:grid-cols-3">
				{#each data.articles as article}
					<ArticleCard {article} categories={data.categories} {tagMap} />
				{/each}
			</div>
		{:else}
			<div class="rounded-xl border-2 border-dashed border-gray-300 p-12 text-center">
				<Tag class="mx-auto h-12 w-12 text-gray-400" />
				<p class="mt-4 text-gray-500">No articles found with this tag</p>
				<p class="mt-2 text-sm text-gray-400">Try browsing categories or search for keywords</p>
			</div>
		{/if}
	{:else if data.query}
		<div class="flex items-center gap-2">
			<Search class="h-5 w-5 text-gray-400" />
			<h1 class="text-xl font-bold text-gray-900">
				Search results for "{data.query}"
			</h1>
			<span class="text-gray-500">({data.articles.length} found)</span>
		</div>

		{#if data.articles.length > 0}
			<div class="grid gap-6 sm:grid-cols-2 lg:grid-cols-3">
				{#each data.articles as article}
					<ArticleCard {article} categories={data.categories} {tagMap} />
				{/each}
			</div>
		{:else}
			<div class="rounded-xl border-2 border-dashed border-gray-300 p-12 text-center">
				<Search class="mx-auto h-12 w-12 text-gray-400" />
				<p class="mt-4 text-gray-500">No articles found matching "{data.query}"</p>
				<p class="mt-2 text-sm text-gray-400">Try different keywords or browse categories</p>
			</div>
		{/if}
	{:else}
		<div class="rounded-xl border-2 border-dashed border-gray-300 p-12 text-center">
			<Search class="mx-auto h-12 w-12 text-gray-400" />
			<p class="mt-4 text-gray-500">Enter a search term to find articles</p>
		</div>
	{/if}
</div>
