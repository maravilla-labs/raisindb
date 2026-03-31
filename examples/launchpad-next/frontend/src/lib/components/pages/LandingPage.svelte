<script lang="ts">
  import { elementComponents } from '$lib/components/elements';
  import type { PageNode, Element } from '$lib/raisin';

  interface Props {
    page: PageNode;
  }

  let { page }: Props = $props();

  // Get elements from properties.content
  const elements: Element[] = $derived(page.properties.content ?? []);
</script>

<article class="landing-page">
  {#each elements as element (element.uuid)}
    {@const Component = elementComponents[element.element_type]}
    {#if Component}
      <Component {element} />
    {:else}
      <div class="unknown-element">
        Unknown element type: {element.element_type}
      </div>
    {/if}
  {/each}
</article>

<style>
  .landing-page {
    min-height: 100vh;
  }

  .unknown-element {
    padding: 2rem;
    background: #fef2f2;
    color: #991b1b;
    text-align: center;
    border: 1px dashed #fca5a5;
  }
</style>
