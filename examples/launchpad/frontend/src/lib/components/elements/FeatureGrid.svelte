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

  // Icon mapping
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
        {#each element.features as feature}
          {@const Icon = getIcon(feature.icon)}
          <div class="feature-card">
            <div class="icon">
              <Icon size={32} />
            </div>
            <h3 class="title">{feature.title}</h3>
            <p class="description">{feature.description}</p>
          </div>
        {/each}
      </div>
    {/if}
  </div>
</section>

<style>
  .feature-grid {
    padding: 4rem 2rem;
    background: #f9fafb;
  }

  .container {
    max-width: 1200px;
    margin: 0 auto;
  }

  .heading {
    font-size: 2rem;
    font-weight: 600;
    text-align: center;
    margin: 0 0 3rem;
    color: #1f2937;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
    gap: 2rem;
  }

  .feature-card {
    background: white;
    padding: 2rem;
    border-radius: 1rem;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.1);
    transition: transform 0.2s, box-shadow 0.2s;
  }

  .feature-card:hover {
    transform: translateY(-4px);
    box-shadow: 0 10px 20px rgba(0, 0, 0, 0.1);
  }

  .icon {
    width: 64px;
    height: 64px;
    background: linear-gradient(135deg, #6366f1 0%, #8b5cf6 100%);
    border-radius: 1rem;
    display: flex;
    align-items: center;
    justify-content: center;
    color: white;
    margin-bottom: 1.5rem;
  }

  .title {
    font-size: 1.25rem;
    font-weight: 600;
    margin: 0 0 0.75rem;
    color: #1f2937;
  }

  .description {
    font-size: 1rem;
    line-height: 1.6;
    color: #6b7280;
    margin: 0;
  }
</style>
