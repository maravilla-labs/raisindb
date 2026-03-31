<script lang="ts">
	import { Eye, Clock, Star } from 'lucide-svelte';
	import { type Article, type Category, type TagNode, getArticleUrl, getCategoryUrl, getCategoryFromPath } from '$lib/types';
	import { formatRelativeDate, formatNumber, truncate } from '$lib/utils';
	import TagChip from './TagChip.svelte';

	export let article: Article;
	export let categories: Category[] = [];
	export let tagMap: Map<string, TagNode> = new Map();
	export let featured = false;

	// Find category from article path
	const categorySlug = getCategoryFromPath(article.path);
	const category = categories.find((c) => c.slug === categorySlug);
</script>

<article
	class="group relative flex flex-col overflow-hidden rounded-xl border border-gray-200 bg-white transition-all hover:border-gray-300 hover:shadow-lg {featured
		? 'md:flex-row'
		: ''}"
>
	{#if article.properties.imageUrl}
		<div class="{featured ? 'md:w-1/2' : ''} aspect-video overflow-hidden bg-gray-100">
			<img
				src={article.properties.imageUrl}
				alt={article.properties.title}
				class="h-full w-full object-cover transition-transform duration-300 group-hover:scale-105"
			/>
		</div>
	{/if}

	<div class="flex flex-1 flex-col p-5 {featured ? 'md:p-8' : ''}">
		<div class="mb-3 flex items-center gap-2">
			{#if category}
				<a
					href={getCategoryUrl(category)}
					class="inline-flex items-center rounded-full px-2.5 py-1 text-xs font-semibold uppercase tracking-wide"
					style="background-color: {category.properties.color}20; color: {category.properties.color}"
				>
					{category.properties.label}
				</a>
			{/if}
			{#if article.properties.featured}
				<span class="inline-flex items-center gap-1 text-xs text-amber-600">
					<Star class="h-3 w-3 fill-current" />
					Featured
				</span>
			{/if}
		</div>

		<h2 class="{featured ? 'text-2xl md:text-3xl' : 'text-lg'} font-bold text-gray-900">
			<a href={getArticleUrl(article)} class="hover:text-blue-600">
				{article.properties.title}
			</a>
		</h2>

		{#if article.properties.excerpt}
			<p class="mt-2 text-gray-600 {featured ? 'text-base' : 'text-sm'}">
				{truncate(article.properties.excerpt, featured ? 200 : 120)}
			</p>
		{/if}

		{#if article.properties.tags && article.properties.tags.length > 0}
			<div class="mt-3 flex flex-wrap gap-1.5">
				{#each article.properties.tags.slice(0, 3) as tag}
					<TagChip
						{tag}
						tagData={tagMap.get(tag['raisin:path'])}
						href="/search?tag={encodeURIComponent(tag['raisin:path'])}"
					/>
				{/each}
				{#if article.properties.tags.length > 3}
					<span class="text-xs text-gray-400">+{article.properties.tags.length - 3} more</span>
				{/if}
			</div>
		{/if}

		<div class="mt-auto pt-4">
			<div class="flex items-center gap-4 text-xs text-gray-500">
				{#if article.properties.author}
					<span class="font-medium text-gray-700">{article.properties.author}</span>
				{/if}
				<span class="flex items-center gap-1">
					<Clock class="h-3.5 w-3.5" />
					{formatRelativeDate(article.properties.publishing_date || article.created_at)}
				</span>
				<span class="flex items-center gap-1">
					<Eye class="h-3.5 w-3.5" />
					{formatNumber(article.properties.views)} views
				</span>
			</div>
		</div>
	</div>
</article>
