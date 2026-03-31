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
  <!-- Ambient glow -->
  <div class="hero-ambient"></div>

  <div class="hero-content">
    {#if element.headline}
      <h1 class="headline">{element.headline}</h1>
    {/if}

    {#if element.subheadline}
      <p class="subheadline">{element.subheadline}</p>
    {/if}

    {#if element.cta_text}
      <div class="cta-wrap">
        {#if element.cta_action}
          <button class="cta-button" onclick={handleActionClick}>
            <span class="cta-text">{element.cta_text}</span>
            <span class="cta-arrow">&rarr;</span>
          </button>
        {:else if element.cta_link}
          <a href={element.cta_link} class="cta-button">
            <span class="cta-text">{element.cta_text}</span>
            <span class="cta-arrow">&rarr;</span>
          </a>
        {/if}
      </div>
    {/if}
  </div>

  <!-- Decorative bottom edge -->
  <div class="hero-edge"></div>
</section>

<style>
  .hero {
    position: relative;
    min-height: 80vh;
    display: flex;
    align-items: center;
    justify-content: center;
    background-color: var(--color-bg);
    background-size: cover;
    background-position: center;
    color: var(--color-text-heading);
    text-align: center;
    padding: 6rem 2rem 5rem;
    overflow: hidden;
  }

  /* Gradient overlay for background images */
  .hero::before {
    content: '';
    position: absolute;
    inset: 0;
    background: linear-gradient(
      180deg,
      rgba(10, 10, 11, 0.7) 0%,
      rgba(10, 10, 11, 0.4) 40%,
      rgba(10, 10, 11, 0.85) 100%
    );
    z-index: 1;
  }

  /* Radial ambient glow */
  .hero-ambient {
    position: absolute;
    top: 20%;
    left: 50%;
    transform: translateX(-50%);
    width: 600px;
    height: 400px;
    background: radial-gradient(ellipse, rgba(212, 175, 55, 0.08) 0%, transparent 70%);
    z-index: 1;
    pointer-events: none;
  }

  .hero-content {
    position: relative;
    z-index: 2;
    max-width: 720px;
    animation: fadeInUp 0.8s ease both;
  }

  .headline {
    font-family: var(--font-display);
    font-size: clamp(2.5rem, 6vw, 4.5rem);
    font-weight: 600;
    margin: 0 0 1.25rem;
    line-height: 1.08;
    letter-spacing: -0.035em;
    color: var(--color-text-heading);
  }

  .subheadline {
    font-size: clamp(1rem, 2vw, 1.2rem);
    color: var(--color-text-secondary);
    margin: 0 0 2.5rem;
    line-height: 1.6;
    max-width: 540px;
    margin-left: auto;
    margin-right: auto;
  }

  .cta-wrap {
    animation: fadeInUp 0.8s ease 0.2s both;
  }

  .cta-button {
    display: inline-flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.875rem 2rem;
    background: var(--color-accent);
    color: var(--color-bg);
    text-decoration: none;
    font-weight: 600;
    font-size: 0.875rem;
    border: none;
    border-radius: var(--radius-sm);
    cursor: pointer;
    transition: all 0.25s ease;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    font-family: var(--font-body);
  }

  .cta-arrow {
    transition: transform 0.25s ease;
    font-size: 1rem;
  }

  .cta-button:hover {
    background: var(--color-accent-hover);
    box-shadow: 0 8px 32px rgba(212, 175, 55, 0.25);
    transform: translateY(-1px);
  }

  .cta-button:hover .cta-arrow {
    transform: translateX(4px);
  }

  /* Thin gold line at bottom */
  .hero-edge {
    position: absolute;
    bottom: 0;
    left: 0;
    right: 0;
    height: 1px;
    background: linear-gradient(
      90deg,
      transparent 0%,
      var(--color-accent) 50%,
      transparent 100%
    );
    opacity: 0.3;
    z-index: 2;
  }

  @media (max-width: 768px) {
    .hero {
      min-height: 60vh;
      padding: 4rem 1.5rem 3rem;
    }

    .hero-ambient {
      width: 300px;
      height: 200px;
    }
  }
</style>
