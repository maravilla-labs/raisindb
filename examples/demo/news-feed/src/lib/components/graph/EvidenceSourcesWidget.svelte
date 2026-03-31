<script lang="ts">
	import type { Article } from '$lib/types';
	import { pathToUrl } from '$lib/types';
	import { FileCheck, ArrowUpRight, BookOpen } from 'lucide-svelte';

	interface Props {
		evidence: Article[];
	}

	let { evidence }: Props = $props();

	const hasEvidence = $derived(evidence.length > 0);
</script>

{#if hasEvidence}
	<div class="rounded-xl border border-green-200 bg-gradient-to-br from-green-50 to-emerald-50 p-5">
		<div class="mb-4 flex items-center gap-2">
			<FileCheck size={20} class="text-green-600" />
			<h3 class="font-semibold text-gray-900">Supporting Evidence</h3>
			<span class="rounded-full bg-green-100 px-2 py-0.5 text-xs font-medium text-green-700">
				{evidence.length} source{evidence.length !== 1 ? 's' : ''}
			</span>
		</div>

		<p class="mb-4 text-sm text-gray-600">
			These articles provide supporting data, research, or additional context for the claims made in this story.
		</p>

		<div class="space-y-3">
			{#each evidence as article}
				<a
					href={pathToUrl(article.path)}
					class="group flex items-start gap-3 rounded-lg border border-green-100 bg-white p-3 transition-all hover:border-green-200 hover:shadow-sm"
				>
					<div class="rounded-lg bg-green-100 p-2">
						<BookOpen size={18} class="text-green-600" />
					</div>
					<div class="min-w-0 flex-1">
						<div class="mb-1 flex items-center gap-2">
							<span class="rounded bg-green-100 px-1.5 py-0.5 text-xs font-medium text-green-700">
								Evidence
							</span>
						</div>
						<p class="font-medium text-gray-900 group-hover:text-green-600">
							{article.properties.title}
						</p>
						{#if article.properties.excerpt}
							<p class="mt-1 line-clamp-2 text-sm text-gray-500">
								{article.properties.excerpt}
							</p>
						{/if}
					</div>
					<ArrowUpRight size={16} class="shrink-0 text-gray-400 group-hover:text-green-600" />
				</a>
			{/each}
		</div>

		<div class="mt-4 flex items-start gap-2 rounded-lg bg-green-100/50 p-3">
			<FileCheck size={16} class="mt-0.5 shrink-0 text-green-600" />
			<p class="text-xs text-green-700">
				<strong>Verification:</strong> These sources have been editorially linked as supporting evidence.
				Always verify claims independently when possible.
			</p>
		</div>
	</div>
{/if}
