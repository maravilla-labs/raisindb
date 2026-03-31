<script lang="ts">
	import type { Page } from '$lib/types';
	import { elementComponents } from '$lib/components/elements/index';

	let { page }: { page: Page } = $props();

	const elements = $derived(page.properties.content ?? []);
</script>

{#each elements as element, i (i)}
	{@const Component = elementComponents[element.element_type]}
	{#if Component}
		<Component {element} />
	{:else}
		<div class="section">
			<div class="container">
				<p class="empty">Unknown block type: {element.element_type}</p>
			</div>
		</div>
	{/if}
{/each}
