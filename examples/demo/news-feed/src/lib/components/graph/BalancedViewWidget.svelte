<script lang="ts">
	import type { Article } from '$lib/types';
	import { pathToUrl } from '$lib/types';
	import { Scale, XCircle, ArrowUpRight } from 'lucide-svelte';

	interface Props {
		opposingViews: Article[];
	}

	let { opposingViews }: Props = $props();

	const hasOpposing = $derived(opposingViews.length > 0);
</script>

{#if hasOpposing}
	<div class="rounded-xl border border-red-200 bg-gradient-to-br from-red-50 to-orange-50 p-5">
		<div class="mb-4 flex items-center gap-2">
			<Scale size={20} class="text-red-600" />
			<h3 class="font-semibold text-gray-900">Balanced View</h3>
		</div>

		<p class="mb-4 text-sm text-gray-600">
			Consider these alternative perspectives and opposing viewpoints for a more complete understanding.
		</p>

		<div class="space-y-3">
			{#each opposingViews as article}
				<a
					href={pathToUrl(article.path)}
					class="group flex items-start gap-3 rounded-lg border border-red-100 bg-white p-3 transition-all hover:border-red-200 hover:shadow-sm"
				>
					<div class="rounded-lg bg-red-100 p-2">
						<XCircle size={18} class="text-red-600" />
					</div>
					<div class="min-w-0 flex-1">
						<div class="mb-1 flex items-center gap-2">
							<span class="rounded bg-red-100 px-1.5 py-0.5 text-xs font-medium text-red-700">
								Opposing View
							</span>
						</div>
						<!-- <pre>
							{JSON.stringify(article, null, 2)}
						</pre> -->
						<p class="font-medium text-gray-900 group-hover:text-red-600">
							{article.properties.title}
						</p>
						{#if article.properties.excerpt}
							<p class="mt-1 line-clamp-2 text-sm text-gray-500">
								{article.properties.excerpt}
							</p>
						{/if}
					</div>
					<ArrowUpRight size={16} class="shrink-0 text-gray-400 group-hover:text-red-600" />
				</a>
			{/each}
		</div>

		<div class="mt-4 rounded-lg bg-amber-50 p-3">
			<p class="text-xs text-amber-700">
				<strong>Why this matters:</strong> Exposure to diverse viewpoints helps readers form well-rounded opinions
				and understand the full complexity of issues.
			</p>
		</div>
	</div>
{/if}
