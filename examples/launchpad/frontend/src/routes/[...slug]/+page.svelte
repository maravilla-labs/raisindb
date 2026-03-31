<script lang="ts">
  import { pageComponents } from '$lib/components/pages';
  import { setCurrentPageContext } from '$lib/stores/navigation';
  import { onDestroy } from 'svelte';
  import type { PageData } from './$types';

  let { data }: { data: PageData } = $props();

  // Get page component based on archetype
  const PageComponent = $derived(
    data.page?.archetype ? pageComponents[data.page.archetype] : undefined
  );

  // Set current page context for AI voice commands
  $effect(() => {
    if (data.page) {
      setCurrentPageContext({
        nodePath: data.page.path,
        nodeType: data.page.node_type,
        archetype: data.page.archetype,
        title: data.page.properties?.title,
      });
    } else {
      setCurrentPageContext(null);
    }
  });

  // Clear context when leaving page
  onDestroy(() => {
    setCurrentPageContext(null);
  });
</script>

<svelte:head>
  {#if data.page}
    <title>{data.page.properties.title} - Launchpad</title>
    {#if data.page.properties.description}
      <meta name="description" content={data.page.properties.description} />
    {/if}
  {:else}
    <title>Page Not Found - Launchpad</title>
  {/if}
</svelte:head>

{#if data.error}
  <div class="error-page">
    <h1>Error</h1>
    <p>{data.error}</p>
    <a href="/">Go Home</a>
  </div>
{:else if !data.page}
  <div class="not-found">
    <h1>Page Not Found</h1>
    <p>The page you're looking for doesn't exist.</p>
    <a href="/">Go Home</a>
  </div>
{:else if PageComponent}
  <PageComponent page={data.page} />
{:else}
  <div class="no-template">
    <h1>{data.page.properties.title}</h1>
    <p>No template found for archetype: {data.page.archetype || 'none'}</p>
    <pre>{JSON.stringify(data.page, null, 2)}</pre>
  </div>
{/if}

<style>
  .error-page,
  .not-found,
  .no-template {
    text-align: center;
    padding: 4rem 2rem;
    max-width: 600px;
    margin: 0 auto;
  }

  .error-page h1,
  .not-found h1 {
    color: #991b1b;
    margin-bottom: 1rem;
  }

  .no-template h1 {
    color: #1f2937;
    margin-bottom: 1rem;
  }

  p {
    color: #6b7280;
    margin-bottom: 2rem;
  }

  a {
    display: inline-block;
    padding: 0.75rem 1.5rem;
    background: #6366f1;
    color: white;
    text-decoration: none;
    border-radius: 0.5rem;
    font-weight: 500;
  }

  a:hover {
    background: #4f46e5;
  }

  pre {
    text-align: left;
    background: #f3f4f6;
    padding: 1rem;
    border-radius: 0.5rem;
    overflow-x: auto;
    font-size: 0.75rem;
  }
</style>
