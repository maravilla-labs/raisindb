<script lang="ts">
	import ArticleCard from '$lib/components/ArticleCard.svelte';
	import ArticleRow from '$lib/components/ArticleRow.svelte';
	import { Star } from 'lucide-svelte';
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
</script>

<div class="space-y-12">
	{#if data.featured.length > 0}
		<section>
			<div class="mb-6 flex items-center gap-2">
				<Star class="h-5 w-5 text-amber-500" />
				<h2 class="text-xl font-bold text-gray-900">Featured Articles</h2>
			</div>
			<div class="grid gap-6 lg:grid-cols-2">
				{#each data.featured as article, i}
					<div class={i === 0 ? 'lg:col-span-2' : ''}>
						<ArticleCard {article} categories={data.categories} {tagMap} featured={i === 0} />
					</div>
				{/each}
			</div>
		</section>
	{/if}

	<section>
		<h2 class="mb-6 text-xl font-bold text-gray-900">Recent Articles</h2>
		{#if data.recent.length > 0}
			<div class="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
				{#each data.recent as article}
					<ArticleRow {article} categories={data.categories} {tagMap} />
				{/each}
			</div>
		{:else}
			<div class="rounded-xl border-2 border-dashed border-gray-300 p-12 text-center">
				<p class="text-gray-500">No articles yet.</p>
				<a
					href="/articles/new"
					class="mt-4 inline-block text-blue-600 hover:underline"
				>
					Create your first article
				</a>
			</div>
		{/if}
	</section>
</div>
