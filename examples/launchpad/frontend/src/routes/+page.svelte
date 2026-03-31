<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';

  // Redirect root to /home
  onMount(() => {
    goto('/home', { replaceState: true });
  });
</script>

<div class="redirecting">
  <p>Redirecting to home...</p>
</div>

<style>
  .redirecting {
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 50vh;
    color: #6b7280;
  }
</style>
