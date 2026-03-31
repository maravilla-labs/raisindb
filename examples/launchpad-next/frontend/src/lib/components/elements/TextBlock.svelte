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
      <h2 class="heading">
        <span class="heading-accent"></span>
        {element.heading}
      </h2>
    {/if}

    {#if element.content}
      <div class="content">
        {#each (element.content as string).split('\n\n') as paragraph}
          <p>{@html paragraph.replace(/\*\*(.*?)\*\*/g, '<strong>$1</strong>')}</p>
        {/each}
      </div>
    {/if}
  </div>
</section>

<style>
  .text-block {
    padding: 5rem 2rem;
  }

  .container {
    max-width: 720px;
    margin: 0 auto;
  }

  .heading {
    font-family: var(--font-display);
    font-size: 1.75rem;
    font-weight: 600;
    margin: 0 0 2rem;
    color: var(--color-text-heading);
    letter-spacing: -0.02em;
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }

  .heading-accent {
    width: 3px;
    height: 1.2em;
    background: var(--color-accent);
    border-radius: 2px;
    flex-shrink: 0;
  }

  .content {
    font-size: 1.05rem;
    line-height: 1.85;
    color: var(--color-text-secondary);
  }

  .content :global(p) {
    margin: 0 0 1.25rem;
  }

  .content :global(p:last-child) {
    margin-bottom: 0;
  }

  .content :global(strong) {
    color: var(--color-text-heading);
    font-weight: 600;
  }

  @media (max-width: 768px) {
    .text-block {
      padding: 3rem 1.5rem;
    }
  }
</style>
