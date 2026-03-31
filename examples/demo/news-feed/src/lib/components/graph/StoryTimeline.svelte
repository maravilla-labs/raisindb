<script lang="ts">
	import type { Article } from '$lib/types';
	import { pathToUrl } from '$lib/types';
	import { ChevronLeft, ChevronRight, Clock, Newspaper } from 'lucide-svelte';

	interface Props {
		predecessors: Article[];
		successors: Article[];
		currentTitle: string;
	}

	let { predecessors, successors, currentTitle }: Props = $props();

	const hasPredecessors = $derived(predecessors.length > 0);
	const hasSuccessors = $derived(successors.length > 0);
	const hasTimeline = $derived(hasPredecessors || hasSuccessors);
</script>

{#if hasTimeline}
	<div class="rounded-xl border border-blue-200 bg-gradient-to-br from-blue-50 to-indigo-50 p-5">
		<div class="mb-4 flex items-center gap-2">
			<Clock size={20} class="text-blue-600" />
			<h3 class="font-semibold text-gray-900">Story Timeline</h3>
		</div>

		<div class="relative">
			<!-- Timeline line -->
			<div class="absolute left-4 top-0 h-full w-0.5 bg-blue-200"></div>

			<div class="space-y-4">
				<!-- Predecessors (earlier stories) -->
				{#each predecessors as article, i}
					<a
						href={pathToUrl(article.path)}
						class="group relative ml-8 block rounded-lg border border-gray-200 bg-white p-3 shadow-sm transition-all hover:border-blue-300 hover:shadow-md"
					>
						<!-- Timeline dot -->
						<div class="absolute -left-6 top-1/2 -translate-y-1/2">
							<div class="flex h-4 w-4 items-center justify-center rounded-full bg-blue-100 ring-4 ring-white">
								<ChevronLeft size={10} class="text-blue-600" />
							</div>
						</div>
						<div class="flex items-center gap-2">
							<span class="text-xs font-medium uppercase tracking-wide text-blue-600">
								Part {i + 1}
							</span>
						</div>
						<p class="mt-1 font-medium text-gray-900 group-hover:text-blue-600">
							{article.properties.title}
						</p>
						<p class="mt-0.5 text-xs text-gray-500">
							{new Date(article.properties.publishing_date || article.created_at).toLocaleDateString()}
						</p>
					</a>
				{/each}

				<!-- Current article -->
				<div class="relative ml-8 rounded-lg border-2 border-blue-400 bg-blue-50 p-3">
					<!-- Timeline dot (current) -->
					<div class="absolute -left-6 top-1/2 -translate-y-1/2">
						<div class="flex h-5 w-5 items-center justify-center rounded-full bg-blue-600 ring-4 ring-white">
							<Newspaper size={12} class="text-white" />
						</div>
					</div>
					<div class="flex items-center gap-2">
						<span class="rounded bg-blue-600 px-1.5 py-0.5 text-xs font-medium text-white">
							Current
						</span>
					</div>
					<p class="mt-1 font-medium text-gray-900">{currentTitle}</p>
				</div>

				<!-- Successors (later stories) -->
				{#each successors as article, i}
					<a
						href={pathToUrl(article.path)}
						class="group relative ml-8 block rounded-lg border border-gray-200 bg-white p-3 shadow-sm transition-all hover:border-blue-300 hover:shadow-md"
					>
						<!-- Timeline dot -->
						<div class="absolute -left-6 top-1/2 -translate-y-1/2">
							<div class="flex h-4 w-4 items-center justify-center rounded-full bg-blue-100 ring-4 ring-white">
								<ChevronRight size={10} class="text-blue-600" />
							</div>
						</div>
						<div class="flex items-center gap-2">
							<span class="text-xs font-medium uppercase tracking-wide text-blue-600">
								Part {predecessors.length + 2 + i}
							</span>
						</div>
						<p class="mt-1 font-medium text-gray-900 group-hover:text-blue-600">
							{article?.properties?.title}
						</p>
						<!-- <pre>
							{JSON.stringify(article, null, 2)}
						</pre> -->
						<p class="mt-0.5 text-xs text-gray-500">
							{new Date(article.properties.publishing_date || article.created_at).toLocaleDateString()}
						</p>
					</a>
				{/each}
			</div>
		</div>
	</div>
{/if}
