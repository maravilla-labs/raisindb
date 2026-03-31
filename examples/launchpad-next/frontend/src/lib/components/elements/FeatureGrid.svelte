<script lang="ts">
  import { Zap, Shield, Globe, Rocket, Code, Users, Star, Heart } from 'lucide-svelte';
  import type { Element } from '$lib/raisin';

  interface Feature {
    icon?: string;
    title: string;
    description: string;
  }

  interface FeatureGridElement extends Element {
    heading?: string;
    features?: Feature[];
  }

  interface Props {
    element: FeatureGridElement;
  }

  let { element }: Props = $props();

  const iconMap: Record<string, typeof Zap> = {
    zap: Zap,
    shield: Shield,
    globe: Globe,
    rocket: Rocket,
    code: Code,
    users: Users,
    star: Star,
    heart: Heart
  };

  function getIcon(name?: string) {
    if (!name) return Star;
    return iconMap[name.toLowerCase()] || Star;
  }
</script>

<section class="feature-grid">
  <div class="container">
    {#if element.heading}
      <h2 class="heading">{element.heading}</h2>
    {/if}

    {#if element.features && element.features.length > 0}
      <div class="grid">
        {#each element.features as feature, i}
          {@const Icon = getIcon(feature.icon)}
          <div class="feature-card" style="animation-delay: {i * 0.08}s">
            <div class="card-inner">
              <div class="icon-wrap">
                <Icon size={24} />
              </div>
              <h3 class="title">{feature.title}</h3>
              <p class="description">{feature.description}</p>
            </div>
            <div class="card-border"></div>
          </div>
        {/each}
      </div>
    {/if}
  </div>
</section>

<style>
  .feature-grid {
    padding: 5rem 2rem;
    position: relative;
  }

  .container {
    max-width: 1100px;
    margin: 0 auto;
  }

  .heading {
    font-family: var(--font-display);
    font-size: 2rem;
    font-weight: 600;
    text-align: center;
    margin: 0 0 3.5rem;
    color: var(--color-text-heading);
    letter-spacing: -0.025em;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
    gap: 1px;
  }

  .feature-card {
    position: relative;
    background: var(--color-bg-card);
    animation: fadeInUp 0.6s ease both;
    overflow: hidden;
  }

  .feature-card:hover {
    background: var(--color-bg-card-hover);
  }

  .feature-card:hover .card-border {
    opacity: 1;
  }

  .feature-card:hover .icon-wrap {
    color: var(--color-accent);
    border-color: var(--color-border-accent);
    background: var(--color-accent-muted);
  }

  .card-inner {
    padding: 2.25rem 2rem;
    position: relative;
    z-index: 1;
  }

  /* Subtle border glow on hover */
  .card-border {
    position: absolute;
    inset: 0;
    border: 1px solid var(--color-border-accent);
    opacity: 0;
    transition: opacity 0.3s ease;
    pointer-events: none;
  }

  .icon-wrap {
    width: 44px;
    height: 44px;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--color-text-secondary);
    margin-bottom: 1.25rem;
    transition: all 0.3s ease;
  }

  .title {
    font-family: var(--font-display);
    font-size: 1.1rem;
    font-weight: 600;
    margin: 0 0 0.625rem;
    color: var(--color-text-heading);
    letter-spacing: -0.01em;
  }

  .description {
    font-size: 0.9rem;
    line-height: 1.6;
    color: var(--color-text-secondary);
    margin: 0;
  }

  @media (max-width: 768px) {
    .feature-grid {
      padding: 3rem 1.5rem;
    }

    .grid {
      grid-template-columns: 1fr;
    }
  }
</style>
