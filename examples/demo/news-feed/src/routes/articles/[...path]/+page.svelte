<script lang="ts">
	import { Eye, Clock, Edit, Trash2, ArrowLeft, FolderOpen, Network } from 'lucide-svelte';
	import { goto } from '$app/navigation';
	import { getArticleUrl, getCategoryUrl, getCategoryFromPath, pathToUrl, ARTICLES_PATH, type TagNode } from '$lib/types';
	import { formatDate, formatNumber } from '$lib/utils';
	import { toasts } from '$lib/stores/toast';
	import MarkdownPreview from '$lib/components/MarkdownPreview.svelte';
	import TagChip from '$lib/components/TagChip.svelte';
	import ArticleCard from '$lib/components/ArticleCard.svelte';
	import {
		CorrectionBanner,
		StoryTimeline,
		SmartRelatedArticles,
		BalancedViewWidget,
		EvidenceSourcesWidget
	} from '$lib/components/graph';

	let { data } = $props();

	// Check if we have any graph data to show
	const hasGraphData = $derived(
		data.type === 'article' && data.graphData && (
			data.graphData.correction ||
			data.graphData.correctsArticle ||
			data.graphData.timeline.predecessors.length > 0 ||
			data.graphData.timeline.successors.length > 0 ||
			data.graphData.smartRelated.length > 0 ||
			data.graphData.opposingViews.length > 0 ||
			data.graphData.evidence.length > 0
		)
	);

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

	let isDeleting = $state(false);

	async function handleDelete() {
		if (data.type !== 'article') return;
		if (!confirm('Are you sure you want to delete this article?')) return;

		isDeleting = true;
		try {
			// Extract the relative path (category/slug) from the full database path
			const relativePath = data.article.path.replace(`${ARTICLES_PATH}/`, '');
			const response = await fetch(`/api/articles/${relativePath}`, {
				method: 'DELETE'
			});

			if (response.ok) {
				toasts.show('success', 'Article deleted successfully');
				goto('/');
			} else {
				const result = await response.json();
				toasts.show('error', result.error || 'Failed to delete article');
			}
		} catch {
			toasts.show('error', 'Failed to delete article');
		} finally {
			isDeleting = false;
		}
	}

	// Compute page title and description based on data type
	const pageTitle = $derived(
		data.type === 'article'
			? `${data.article.properties.title} - News Feed`
			: `${data.category.properties.label} - News Feed`
	);
	const pageDescription = $derived(
		data.type === 'article' ? data.article.properties.excerpt : ''
	);
</script>

<svelte:head>
	<title>{pageTitle}</title>
	{#if pageDescription}
		<meta name="description" content={pageDescription} />
	{/if}
</svelte:head>

{#if data.type === 'article'}
	<!-- Article Detail View -->

	<div class="mx-auto max-w-7xl">
		<div class="mb-8">
			<a href="/" class="inline-flex items-center gap-1 text-sm text-gray-500 hover:text-gray-700">
				<ArrowLeft class="h-4 w-4" />
				Back to feed
			</a>
		</div>

		<!-- Correction Banners -->
		{#if data.graphData?.correction || data.graphData?.correctsArticle}
			<CorrectionBanner
				correction={data.graphData.correction}
				correctsArticle={data.graphData.correctsArticle}
			/>
		{/if}

		<!-- Two-column layout when graph data exists -->
		<div class="flex flex-col gap-8 lg:flex-row">
			<!-- Main Content Column -->
			<article class="min-w-0 flex-1">
				{#if data.article.properties.imageUrl}
					<div class="mb-8 overflow-hidden rounded-2xl">
						<img
							src={data.article.properties.imageUrl}
							alt={data.article.properties.title}
							class="aspect-video w-full object-cover"
						/>
					</div>
				{/if}

				<header class="mb-8">
					{#if data.category}
						<a
							href={getCategoryUrl(data.category)}
							class="mb-4 inline-flex items-center rounded-full px-3 py-1 text-sm font-semibold uppercase tracking-wide"
							style="background-color: {data.category.properties.color}20; color: {data.category.properties.color}"
						>
							{data.category.properties.label}
						</a>
					{/if}

					<h1 class="text-3xl font-bold text-gray-900 sm:text-4xl">
						{data.article.properties.title}
					</h1>

					{#if data.article.properties.excerpt}
						<p class="mt-4 text-xl text-gray-600">
							{data.article.properties.excerpt}
						</p>
					{/if}

					<div class="mt-6 flex flex-wrap items-center gap-4 text-sm text-gray-500">
						{#if data.article.properties.author}
							<span class="font-medium text-gray-700">{data.article.properties.author}</span>
						{/if}
						<span class="flex items-center gap-1">
							<Clock class="h-4 w-4" />
							{formatDate(data.article.properties.publishing_date || data.article.created_at)}
						</span>
						<span class="flex items-center gap-1">
							<Eye class="h-4 w-4" />
							{formatNumber(data.article.properties.views)} views
						</span>
						{#if hasGraphData}
							<span class="flex items-center gap-1 text-purple-600">
								<Network class="h-4 w-4" />
								Graph Connected
							</span>
						{/if}
					</div>

					{#if data.article.properties.tags && data.article.properties.tags.length > 0}
						<div class="mt-4 flex flex-wrap gap-2">
							{#each data.article.properties.tags as tag}
								<TagChip
									{tag}
									tagData={tagMap.get(tag['raisin:path'])}
									href="/search?tag={encodeURIComponent(tag['raisin:path'])}"
									size="md"
								/>
							{/each}
						</div>
					{/if}
				</header>

				<div class="prose max-w-none">
					<MarkdownPreview content={data.article.properties.body} />
				</div>

				<div class="mt-12 flex items-center gap-4 border-t border-gray-200 pt-8">
					<a
						href="{getArticleUrl(data.article)}/edit"
						class="inline-flex items-center gap-2 rounded-lg border border-gray-300 px-4 py-2 text-sm font-medium text-gray-700 hover:bg-gray-50"
					>
						<Edit class="h-4 w-4" />
						Edit
					</a>
					<a
						href="{getArticleUrl(data.article)}/move"
						class="inline-flex items-center gap-2 rounded-lg border border-gray-300 px-4 py-2 text-sm font-medium text-gray-700 hover:bg-gray-50"
					>
						<FolderOpen class="h-4 w-4" />
						Move
					</a>
					<button
						onclick={handleDelete}
						disabled={isDeleting}
						class="inline-flex items-center gap-2 rounded-lg border border-red-300 px-4 py-2 text-sm font-medium text-red-700 hover:bg-red-50 disabled:opacity-50"
					>
						<Trash2 class="h-4 w-4" />
						{isDeleting ? 'Deleting...' : 'Delete'}
					</button>
				</div>
			</article>

			<!-- Graph Sidebar (when graph data exists) -->
			{#if hasGraphData}
			
				<aside class="w-full shrink-0 space-y-6 lg:w-96">
					<!-- Story Timeline -->
					{#if data.graphData && (data.graphData.timeline.predecessors.length > 0 || data.graphData.timeline.successors.length > 0)}
						<StoryTimeline
							predecessors={data.graphData.timeline.predecessors}
							successors={data.graphData.timeline.successors}
							currentTitle={data.article.properties.title}
						/>
					{/if}

					<!-- Balanced View (Contradictions) -->
					{#if data.graphData && data.graphData.opposingViews.length > 0}
						<BalancedViewWidget opposingViews={data.graphData.opposingViews} />
					{/if}

					<!-- Evidence Sources -->
					{#if data.graphData && data.graphData.evidence.length > 0}
						<EvidenceSourcesWidget evidence={data.graphData.evidence} />
					{/if}

					<!-- Smart Related -->
					{#if data.graphData && data.graphData.smartRelated.length > 0}
						<SmartRelatedArticles related={data.graphData.smartRelated} />
					{/if}
				</aside>
			{/if}
		</div>
	</div>

	{#if data.related.length > 0}
		<section class="mx-auto mt-10 max-w-7xl border-t border-gray-200 pt-12">
			<h2 class="mb-6 text-xl font-bold text-gray-900">More in this Category</h2>
			<div class="grid gap-6 sm:grid-cols-2 lg:grid-cols-3">
				{#each data.related as article}
					<ArticleCard {article} categories={data.categories} {tagMap} />
				{/each}
			</div>
		</section>
	{/if}

{:else if data.type === 'category'}
	<!-- Category Listing View -->
	<div class="space-y-8">
		<div class="flex items-center gap-4">
			<div
				class="h-12 w-12 rounded-xl"
				style="background-color: {data.category.properties.color}"
			></div>
			<div>
				<h1 class="text-2xl font-bold text-gray-900">{data.category.properties.label}</h1>
				<p class="text-gray-500">{data.articles.length} articles</p>
			</div>
		</div>

		{#if data.articles.length > 0}
			<div class="grid gap-6 sm:grid-cols-2 lg:grid-cols-3">
				{#each data.articles as article}
					<ArticleCard {article} categories={data.categories} {tagMap} />
				{/each}
			</div>
		{:else}
			<div class="rounded-xl border-2 border-dashed border-gray-300 p-12 text-center">
				<p class="text-gray-500">No articles in this category yet.</p>
				<a href="/articles/new" class="mt-4 inline-block text-blue-600 hover:underline">
					Create the first one
				</a>
			</div>
		{/if}
	</div>
{/if}
