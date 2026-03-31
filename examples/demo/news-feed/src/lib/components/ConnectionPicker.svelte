<script lang="ts">
	import type { Article, ArticleConnection } from '$lib/types';
	import ConnectionCard from './ConnectionCard.svelte';
	import ConnectionModal from './ConnectionModal.svelte';
	import { Plus, Network } from 'lucide-svelte';

	interface Props {
		connections: ArticleConnection[];
		availableArticles: Article[];
		currentPath: string;
		onchange?: (connections: ArticleConnection[]) => void;
	}

	let { connections = $bindable([]), availableArticles, currentPath, onchange }: Props = $props();

	let modalOpen = $state(false);
	let editingConnection = $state<ArticleConnection | null>(null);
	let editingIndex = $state<number | null>(null);

	// Filter out current article and already connected articles from available list
	const filteredAvailableArticles = $derived(() => {
		const connectedPaths = new Set(connections.map(c => c.targetPath));
		return availableArticles.filter(a =>
			a.path !== currentPath &&
			(editingConnection ? true : !connectedPaths.has(a.path))
		);
	});

	function handleAddConnection() {
		editingConnection = null;
		editingIndex = null;
		modalOpen = true;
	}

	function handleEditConnection(index: number) {
		editingConnection = connections[index];
		editingIndex = index;
		modalOpen = true;
	}

	function handleRemoveConnection(index: number) {
		const newConnections = [...connections];
		newConnections.splice(index, 1);
		connections = newConnections;
		onchange?.(connections);
	}

	function handleSaveConnection(connection: ArticleConnection) {
		if (editingIndex !== null) {
			// Update existing connection
			const newConnections = [...connections];
			newConnections[editingIndex] = connection;
			connections = newConnections;
		} else {
			// Add new connection
			connections = [...connections, connection];
		}
		onchange?.(connections);
		modalOpen = false;
		editingConnection = null;
		editingIndex = null;
	}

	function handleModalClose() {
		modalOpen = false;
		editingConnection = null;
		editingIndex = null;
	}
</script>

<div class="space-y-4">
	<!-- Header -->
	<div class="flex items-center justify-between">
		<div class="flex items-center gap-2">
			<Network size={20} class="text-gray-500" />
			<h3 class="text-sm font-semibold text-gray-900">Story Connections</h3>
			{#if connections.length > 0}
				<span class="rounded-full bg-blue-100 px-2 py-0.5 text-xs font-medium text-blue-700">
					{connections.length}
				</span>
			{/if}
		</div>
		<button
			type="button"
			onclick={handleAddConnection}
			class="inline-flex items-center gap-1.5 rounded-lg bg-blue-600 px-3 py-1.5 text-sm font-medium text-white transition-colors hover:bg-blue-700"
		>
			<Plus size={16} />
			Add Connection
		</button>
	</div>

	<!-- Description -->
	<p class="text-sm text-gray-500">
		Create semantic connections to other articles. These relationships power smart recommendations and help readers discover related content.
	</p>

	<!-- Connection list -->
	{#if connections.length > 0}
		<div class="space-y-2">
			{#each connections as connection, index}
				<ConnectionCard
					{connection}
					onedit={() => handleEditConnection(index)}
					onremove={() => handleRemoveConnection(index)}
				/>
			{/each}
		</div>
	{:else}
		<div class="rounded-lg border-2 border-dashed border-gray-200 bg-gray-50 px-6 py-8 text-center">
			<Network size={32} class="mx-auto mb-2 text-gray-400" />
			<p class="text-sm font-medium text-gray-900">No connections yet</p>
			<p class="mt-1 text-xs text-gray-500">
				Add connections to help readers discover related stories
			</p>
		</div>
	{/if}
</div>

<!-- Connection Modal -->
<ConnectionModal
	bind:isOpen={modalOpen}
	availableArticles={filteredAvailableArticles()}
	{editingConnection}
	{currentPath}
	onclose={handleModalClose}
	onsave={handleSaveConnection}
/>
