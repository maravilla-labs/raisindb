<script lang="ts">
  import { triggerAction } from '$lib/stores/actions';
  import type { Element } from '$lib/raisin';

  interface HeroElement extends Element {
    headline?: string;
    subheadline?: string;
    cta_text?: string;
    cta_link?: string;
    cta_action?: string;
    background_image?: {
      url?: string;
    };
  }

  interface Props {
    element: HeroElement;
  }

  let { element }: Props = $props();

  function handleActionClick() {
    if (element.cta_action) {
      triggerAction(element.cta_action);
    }
  }
</script>

<section
  class="hero"
  style:background-image={element.background_image?.url ? `url(${element.background_image.url})` : undefined}
>
  <div class="hero-content">
    {#if element.headline}
      <h1 class="headline">{element.headline}</h1>
    {/if}

    {#if element.subheadline}
      <p class="subheadline">{element.subheadline}</p>
    {/if}

    {#if element.cta_text}
      {#if element.cta_action}
        <button class="cta-button" onclick={handleActionClick}>
          {element.cta_text}
        </button>
      {:else if element.cta_link}
        <a href={element.cta_link} class="cta-button">
          {element.cta_text}
        </a>
      {/if}
    {/if}
  </div>
</section>

<style>
  .hero {
    min-height: 60vh;
    display: flex;
    align-items: center;
    justify-content: center;
    background: linear-gradient(135deg, #6366f1 0%, #8b5cf6 100%);
    background-size: cover;
    background-position: center;
    color: white;
    text-align: center;
    padding: 4rem 2rem;
  }

  .hero-content {
    max-width: 800px;
  }

  .headline {
    font-size: 3rem;
    font-weight: 700;
    margin: 0 0 1rem;
    line-height: 1.2;
  }

  .subheadline {
    font-size: 1.25rem;
    opacity: 0.9;
    margin: 0 0 2rem;
    line-height: 1.6;
  }

  .cta-button {
    display: inline-block;
    padding: 1rem 2rem;
    background: white;
    color: #6366f1;
    text-decoration: none;
    font-weight: 600;
    font-size: 1rem;
    border: none;
    border-radius: 0.5rem;
    cursor: pointer;
    transition: transform 0.2s, box-shadow 0.2s;
  }

  .cta-button:hover {
    transform: translateY(-2px);
    box-shadow: 0 10px 20px rgba(0, 0, 0, 0.2);
  }

  @media (max-width: 768px) {
    .headline {
      font-size: 2rem;
    }

    .subheadline {
      font-size: 1rem;
    }
  }
</style>
