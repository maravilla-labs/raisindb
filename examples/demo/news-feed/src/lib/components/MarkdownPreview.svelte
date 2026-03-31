<script lang="ts">
	import { marked } from 'marked';

	export let content: string;

	$: html = marked(content || '', { async: false }) as string;
</script>

<div class="prose max-w-none">
	{@html html}
</div>
