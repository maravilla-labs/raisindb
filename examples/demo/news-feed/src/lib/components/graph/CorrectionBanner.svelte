<script lang="ts">
	import { AlertTriangle, ArrowRight, CheckCircle } from 'lucide-svelte';
	import { pathToUrl } from '$lib/types';

	interface CorrectionInfo {
		title: string;
		path: string;
		publishedAt: string;
	}

	interface Props {
		// Article that corrects THIS one (incoming edge)
		correction: CorrectionInfo | null;
		// Article that THIS one corrects (outgoing edge)
		correctsArticle?: CorrectionInfo | null;
	}

	let { correction, correctsArticle }: Props = $props();
</script>

{#if correction}
	<div class="mb-6 rounded-xl border border-amber-200 bg-gradient-to-r from-amber-50 to-orange-50 p-4 shadow-sm">
		<div class="flex items-start gap-3">
			<div class="rounded-full bg-amber-100 p-2">
				<AlertTriangle size={20} class="text-amber-600" />
			</div>
			<div class="flex-1">
				<h3 class="font-semibold text-amber-900">Correction Available</h3>
				<p class="mt-1 text-sm text-amber-700">
					This article has been corrected. Please see the updated information below.
				</p>
				<a
					href={pathToUrl(correction.path)}
					class="mt-3 inline-flex items-center gap-2 rounded-lg bg-amber-600 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-amber-700"
				>
					Read Correction
					<ArrowRight size={16} />
				</a>
			</div>
		</div>
	</div>
{/if}

{#if correctsArticle}
	<div class="mb-6 rounded-xl border border-green-200 bg-gradient-to-r from-green-50 to-emerald-50 p-4 shadow-sm">
		<div class="flex items-start gap-3">
			<div class="rounded-full bg-green-100 p-2">
				<CheckCircle size={20} class="text-green-600" />
			</div>
			<div class="flex-1">
				<h3 class="font-semibold text-green-900">This is a Correction</h3>
				<p class="mt-1 text-sm text-green-700">
					This article is a correction of an earlier article. View the original below.
				</p>
				<a
					href={pathToUrl(correctsArticle.path)}
					class="mt-3 inline-flex items-center gap-2 rounded-lg bg-green-600 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-green-700"
				>
					View Original Article
					<ArrowRight size={16} />
				</a>
			</div>
		</div>
	</div>
{/if}
