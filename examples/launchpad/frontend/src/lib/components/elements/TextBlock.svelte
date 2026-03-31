<script lang="ts">
  import type { Element } from '$lib/raisin';

  interface TextBlockElement extends Element {
    heading?: string;
    content?: string;
  }

  interface Props {
    element: TextBlockElement;
  }

  let { element }: Props = $props();
</script>

<section class="text-block">
  <div class="container">
    {#if element.heading}
      <h2 class="heading">{element.heading}</h2>
    {/if}

    {#if element.content}
      <div class="content">
        <!-- Simple markdown-like rendering -->
        {#each (element.content as string).split('\n\n') as paragraph}
          <p>{@html paragraph.replace(/\*\*(.*?)\*\*/g, '<strong>$1</strong>')}</p>
        {/each}
      </div>
    {/if}
  </div>
</section>

<style>
  .text-block {
    padding: 4rem 2rem;
  }

  .container {
    max-width: 800px;
    margin: 0 auto;
  }

  .heading {
    font-size: 2rem;
    font-weight: 600;
    margin: 0 0 1.5rem;
    color: #1f2937;
  }

  .content {
    font-size: 1.125rem;
    line-height: 1.8;
    color: #4b5563;
  }

  .content :global(p) {
    margin: 0 0 1rem;
  }

  .content :global(p:last-child) {
    margin-bottom: 0;
  }

  .content :global(strong) {
    color: #1f2937;
    font-weight: 600;
  }
</style>
