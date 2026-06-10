<script lang="ts">
  import { notifications } from '../stores/notifications.svelte';
</script>

<!-- Small stack of auto-dismissing toasts for incoming inbox items. -->
<div class="toasts" aria-live="polite">
  {#each notifications.toasts as toast (toast.id)}
    <div class="toast">
      <span class="toast-icon">📬</span>
      <span>{toast.title}</span>
    </div>
  {/each}
</div>
