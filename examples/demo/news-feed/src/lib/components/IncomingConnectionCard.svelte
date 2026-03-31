<script lang="ts">
	import type { IncomingConnection } from '$lib/types';
	import { RELATION_TYPE_META, pathToUrl } from '$lib/types';
	import { ArrowRight, RefreshCw, Pencil, XCircle, FileCheck, Link, Bookmark, ExternalLink } from 'lucide-svelte';

	interface Props {
		connection: IncomingConnection;
	}

	let { connection }: Props = $props();

	const meta = $derived(RELATION_TYPE_META[connection.relationType]);

	// Get icon component for relation type
	const iconMap = {
		'arrow-right': ArrowRight,
		'refresh-cw': RefreshCw,
		'pencil': Pencil,
		'x-circle': XCircle,
		'file-check': FileCheck,
		'link': Link,
		'bookmark': Bookmark
	};

	const IconComponent = $derived(iconMap[meta.icon as keyof typeof iconMap]);

	// Get inverse label for incoming connections
	const inverseLabels: Record<string, string> = {
		'continues': 'Continued by',
		'updates': 'Updated by',
		'corrects': 'Corrected by',
		'contradicts': 'Contradicted by',
		'provides-evidence-for': 'Evidence from',
		'similar-to': 'Similar to',
		'see-also': 'Referenced by'
	};

	const inverseLabel = $derived(inverseLabels[connection.relationType] || meta.label);
</script>

<a
	href={pathToUrl(connection.sourcePath)}
	class="group block rounded-lg border border-gray-200 bg-white transition-all hover:border-gray-300 hover:shadow-sm"
>
	<div class="flex items-start gap-3 p-3">
		<!-- Relation type badge (inverse) -->
		<span
			class="mt-0.5 inline-flex shrink-0 items-center gap-1 rounded-full px-2 py-0.5 text-xs font-medium text-white opacity-80"
			style="background-color: {meta.color}"
		>
			{#if IconComponent}
				<IconComponent size={12} />
			{/if}
			{inverseLabel}
		</span>

		<!-- Source article info -->
		<div class="min-w-0 flex-1">
			<p class="truncate font-medium text-gray-900 group-hover:text-blue-600">{connection.sourceTitle}</p>
			<p class="truncate text-xs text-gray-500">{pathToUrl(connection.sourcePath)}</p>

			<!-- Weight bar -->
			<div class="mt-2 flex items-center gap-2">
				<div class="h-1.5 flex-1 overflow-hidden rounded-full bg-gray-200">
					<div
						class="h-full rounded-full opacity-60 transition-all"
						style="width: {connection.weight}%; background-color: {meta.color}"
					></div>
				</div>
				<span class="text-xs font-medium text-gray-500">{connection.weight}%</span>
			</div>
		</div>

		<!-- External link indicator -->
		<div class="shrink-0 self-center text-gray-400 group-hover:text-blue-600">
			<ExternalLink size={16} />
		</div>
	</div>
</a>
