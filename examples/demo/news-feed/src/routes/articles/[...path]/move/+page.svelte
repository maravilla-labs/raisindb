<script lang="ts">
	import { ArrowLeft, FolderOpen } from 'lucide-svelte';
	import { getArticleUrl } from '$lib/types';

	let { data, form } = $props();

	let selectedCategory = $state(data.currentCategory);
	let isSubmitting = $state(false);
</script>

<svelte:head>
	<title>Move: {data.article.properties.title} - News Feed</title>
</svelte:head>

<div class="mx-auto max-w-xl">
	<div class="mb-8">
		<a
			href={getArticleUrl(data.article)}
			class="inline-flex items-center gap-1 text-sm text-gray-500 hover:text-gray-700"
		>
			<ArrowLeft class="h-4 w-4" />
			Back to article
		</a>
	</div>

	<div class="rounded-xl border border-gray-200 bg-white p-6 shadow-sm">
		<div class="mb-6 flex items-center gap-3">
			<FolderOpen class="h-6 w-6 text-gray-400" />
			<h1 class="text-2xl font-bold text-gray-900">Move Article</h1>
		</div>

		<p class="mb-6 text-gray-600">
			Move "<span class="font-medium text-gray-900">{data.article.properties.title}</span>" to a different category.
		</p>

		{#if form?.error}
			<div class="mb-6 rounded-lg bg-red-50 p-4 text-sm text-red-700">
				{form.error}
			</div>
		{/if}

		<form method="POST" class="space-y-6">
			<div>
				<label for="category" class="block text-sm font-medium text-gray-700">
					Select Category
				</label>
				<div class="mt-3 space-y-2">
					{#each data.categories as category}
						<label
							class="flex cursor-pointer items-center gap-3 rounded-lg border p-4 transition-colors {selectedCategory === category.slug
								? 'border-blue-500 bg-blue-50'
								: 'border-gray-200 hover:border-gray-300'}"
						>
							<input
								type="radio"
								name="category"
								value={category.slug}
								bind:group={selectedCategory}
								class="h-4 w-4 border-gray-300 text-blue-600 focus:ring-blue-500"
							/>
							<div
								class="h-4 w-4 rounded"
								style="background-color: {category.properties.color}"
							></div>
							<span class="font-medium text-gray-900">{category.properties.label}</span>
							{#if category.slug === data.currentCategory}
								<span class="ml-auto text-xs text-gray-500">(current)</span>
							{/if}
						</label>
					{/each}
				</div>
			</div>

			<div class="flex items-center justify-end gap-4 border-t border-gray-200 pt-6">
				<a
					href={getArticleUrl(data.article)}
					class="rounded-lg border border-gray-300 px-4 py-2 text-sm font-medium text-gray-700 hover:bg-gray-50"
				>
					Cancel
				</a>
				<button
					type="submit"
					disabled={isSubmitting || selectedCategory === data.currentCategory}
					class="rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700 disabled:opacity-50"
				>
					{isSubmitting ? 'Moving...' : 'Move Article'}
				</button>
			</div>
		</form>
	</div>
</div>
