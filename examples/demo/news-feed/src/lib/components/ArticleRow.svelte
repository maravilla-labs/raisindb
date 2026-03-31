<script lang="ts">
	import { Clock } from 'lucide-svelte';
	import { type Article, type Category, type TagNode, getArticleUrl, getCategoryUrl, getCategoryFromPath } from '$lib/types';
	import { formatRelativeDate, truncate } from '$lib/utils';
	import TagChip from './TagChip.svelte';

	export let article: Article;
	export let categories: Category[] = [];
	export let tagMap: Map<string, TagNode> = new Map();

	const categorySlug = getCategoryFromPath(article.path);
	const category = categories.find((c) => c.slug === categorySlug);
</script>

<article class="group flex gap-4 rounded-lg border border-gray-200 bg-white p-3 transition-all hover:border-gray-300 hover:shadow-md">
	{#if article.properties.imageUrl}
		<div class="h-20 w-28 flex-shrink-0 overflow-hidden rounded-md bg-gray-100">
			<img
				src={article.properties.imageUrl}
				alt={article.properties.title}
				class="h-full w-full object-cover transition-transform duration-300 group-hover:scale-105"
			/>
		</div>
	{/if}

	<div class="flex min-w-0 flex-1 flex-col justify-center">
		<div class="mb-1 flex items-center gap-2">
			{#if category}
				<a
					href={getCategoryUrl(category)}
					class="inline-flex items-center rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide"
					style="background-color: {category.properties.color}20; color: {category.properties.color}"
				>
					{category.properties.label}
				</a>
			{/if}
			<span class="flex items-center gap-1 text-[10px] text-gray-400">
				<Clock class="h-3 w-3" />
				{formatRelativeDate(article.properties.publishing_date || article.created_at)}
			</span>
		</div>

		<h3 class="line-clamp-2 text-sm font-semibold text-gray-900">
			<a href={getArticleUrl(article)} class="hover:text-blue-600">
				{article.properties.title}
			</a>
		</h3>

		{#if article.properties.excerpt}
			<p class="mt-1 line-clamp-1 text-xs text-gray-500">
				{truncate(article.properties.excerpt, 80)}
			</p>
		{/if}

		{#if article.properties.tags && article.properties.tags.length > 0}
			<div class="mt-1.5 flex flex-wrap gap-1">
				{#each article.properties.tags.slice(0, 2) as tag}
					<TagChip
						{tag}
						tagData={tagMap.get(tag['raisin:path'])}
						size="sm"
					/>
				{/each}
				{#if article.properties.tags.length > 2}
					<span class="text-[10px] text-gray-400">+{article.properties.tags.length - 2}</span>
				{/if}
			</div>
		{/if}
	</div>
</article>
