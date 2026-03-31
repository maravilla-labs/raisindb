<script lang="ts">
	import type { Article } from '$lib/types';
	import { pathToUrl, RELATION_TYPE_META, type ArticleRelationType } from '$lib/types';
	import { Sparkles, ArrowRight, RefreshCw, Link, Bookmark } from 'lucide-svelte';

	interface RelatedArticle {
		article: Article;
		weight: number;
		relationType: string;
	}

	interface Props {
		related: RelatedArticle[];
	}

	let { related }: Props = $props();

	const hasRelated = $derived(related.length > 0);

	// Icon map for relation types
	const iconMap = {
		'arrow-right': ArrowRight,
		'refresh-cw': RefreshCw,
		'link': Link,
		'bookmark': Bookmark
	};

	function getIconForType(relationType: string) {
		const meta = RELATION_TYPE_META[relationType as ArticleRelationType];
		if (meta) {
			return iconMap[meta.icon as keyof typeof iconMap] || Link;
		}
		return Link;
	}

	function getColorForType(relationType: string): string {
		const meta = RELATION_TYPE_META[relationType as ArticleRelationType];
		return meta?.color || '#6B7280';
	}

	function getLabelForType(relationType: string): string {
		const meta = RELATION_TYPE_META[relationType as ArticleRelationType];
		return meta?.label || relationType;
	}
</script>

{#if hasRelated}
	<div class="rounded-xl border border-gray-200 bg-white p-5">
		<div class="mb-4 flex items-center gap-2">
			<Sparkles size={20} class="text-purple-500" />
			<h3 class="font-semibold text-gray-900">Smart Related</h3>
			<span class="rounded-full bg-purple-100 px-2 py-0.5 text-xs font-medium text-purple-700">
				{related.length}
			</span>
		</div>

		<p class="mb-4 text-sm text-gray-500">
			Articles connected by editorial relationships, ranked by relevance
		</p>

		<div class="space-y-3">
			{#each related as { article, weight, relationType }}
				{@const color = getColorForType(relationType)}
				{@const IconComponent = getIconForType(relationType)}
				<a
					href={pathToUrl(article.path)}
					class="group block rounded-lg border border-gray-100 bg-gray-50 p-3 transition-all hover:border-gray-200 hover:bg-white hover:shadow-sm"
				>
					<div class="flex items-start gap-3">
						<!-- Relevance indicator -->
						<div
							class="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg text-white"
							style="background-color: {color}"
						>
							<IconComponent size={18} />
						</div>

						<div class="min-w-0 flex-1">
							<div class="mb-1 flex items-center gap-2">
								<span
									class="rounded px-1.5 py-0.5 text-xs font-medium text-white"
									style="background-color: {color}"
								>
									{getLabelForType(relationType)}
								</span>
								<span class="text-xs text-gray-500">{Math.round(weight)}% relevant</span>
							</div>
							<p class="font-medium text-gray-900 group-hover:text-blue-600">
								{article.properties.title}
							</p>
							{#if article.properties.excerpt}
								<p class="mt-1 line-clamp-2 text-sm text-gray-500">
									{article.properties.excerpt}
								</p>
							{/if}
						</div>

						<!-- Weight bar -->
						<div class="w-16 shrink-0">
							<div class="h-2 overflow-hidden rounded-full bg-gray-200">
								<div
									class="h-full rounded-full transition-all"
									style="width: {weight}%; background-color: {color}"
								></div>
							</div>
						</div>
					</div>
				</a>
			{/each}
		</div>
	</div>
{/if}
