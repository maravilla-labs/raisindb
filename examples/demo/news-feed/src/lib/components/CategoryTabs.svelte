<script lang="ts">
	import { page } from '$app/stores';
	import { getCategoryUrl, type Category } from '$lib/types';

	let { categories = [] }: { categories: Category[] } = $props();

	function isActive(category: Category): boolean {
		const categoryUrl = getCategoryUrl(category);
		return $page.url.pathname === categoryUrl || $page.url.pathname.startsWith(categoryUrl + '/');
	}

	function isHome(): boolean {
		return $page.url.pathname === '/';
	}
</script>

<nav class="border-b border-gray-200 bg-white">
	<div class="mx-auto max-w-7xl px-4 sm:px-6 lg:px-8">
		<div class="-mb-px flex space-x-8 overflow-x-auto">
			<a
				href="/"
				class="whitespace-nowrap border-b-2 px-1 py-4 text-sm font-medium transition-colors {isHome()
					? 'border-gray-900 text-gray-900'
					: 'border-transparent text-gray-500 hover:border-gray-300 hover:text-gray-700'}"
			>
				All
			</a>
			{#each categories as category}
				<a
					href={getCategoryUrl(category)}
					class="whitespace-nowrap border-b-2 px-1 py-4 text-sm font-medium transition-colors {isActive(
						category
					)
						? 'border-current'
						: 'border-transparent text-gray-500 hover:border-gray-300 hover:text-gray-700'}"
					style={isActive(category) ? `color: ${category.properties.color}` : ''}
				>
					<span
						class="mr-2 inline-block h-2 w-2 rounded-full"
						style="background-color: {category.properties.color}"
					></span>
					{category.properties.label}
				</a>
			{/each}
		</div>
	</div>
</nav>
