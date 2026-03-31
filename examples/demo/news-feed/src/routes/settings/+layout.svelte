<script lang="ts">
	import { page } from '$app/stores';
	import { FolderOpen, Tag } from 'lucide-svelte';

	const tabs = [
		{ href: '/settings/categories', label: 'Categories', icon: FolderOpen },
		{ href: '/settings/tags', label: 'Tags', icon: Tag }
	];

	let { children } = $props();
</script>

<div class="mx-auto max-w-4xl">
	<div class="mb-8">
		<h1 class="text-2xl font-bold text-gray-900">Settings</h1>
		<p class="mt-1 text-sm text-gray-500">Manage your news feed configuration</p>
	</div>

	<!-- Tabs -->
	<div class="mb-6 border-b border-gray-200">
		<nav class="-mb-px flex gap-6">
			{#each tabs as tab}
				{@const isActive = $page.url.pathname === tab.href || $page.url.pathname.startsWith(tab.href + '/')}
				<a
					href={tab.href}
					class="flex items-center gap-2 border-b-2 px-1 py-3 text-sm font-medium transition-colors {isActive
						? 'border-blue-500 text-blue-600'
						: 'border-transparent text-gray-500 hover:border-gray-300 hover:text-gray-700'}"
				>
					<tab.icon size={16} />
					{tab.label}
				</a>
			{/each}
		</nav>
	</div>

	{@render children()}
</div>
