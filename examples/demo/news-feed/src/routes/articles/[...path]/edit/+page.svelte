<script lang="ts">
	import { ArrowLeft } from 'lucide-svelte';
	import ArticleForm from '$lib/components/ArticleForm.svelte';
	import { getArticleUrl } from '$lib/types';

	let { data, form } = $props();
</script>

<svelte:head>
	<title>Edit: {data.article.properties.title} - News Feed</title>
</svelte:head>

<div class="mx-auto max-w-3xl">
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
		<h1 class="mb-6 text-2xl font-bold text-gray-900">Edit Article</h1>

		{#if form?.error}
			<div class="mb-6 rounded-lg bg-red-50 p-4 text-sm text-red-700">
				{form.error}
			</div>
		{/if}

		<ArticleForm
			article={data.article.properties}
			categories={data.categories}
			availableTags={data.tags ?? []}
			availableArticles={data.availableArticles ?? []}
			incomingConnections={data.incomingConnections ?? []}
			currentPath={data.article.path}
			initialCategory={data.currentCategory}
			submitLabel="Save Changes"
		/>
	</div>
</div>
