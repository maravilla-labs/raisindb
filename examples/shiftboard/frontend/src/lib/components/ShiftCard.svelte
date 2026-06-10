<script lang="ts">
  import type { Shift } from '../stores/board.svelte';

  let { shift }: { shift: Shift } = $props();
</script>

<!-- flashSeq > 0 means this card was just updated by a live event. -->
<article
  class="shift-card"
  class:flash={shift.flashSeq > 0}
  data-testid="shift-card"
  data-path={shift.path}
  data-status={shift.status}
>
  <div class="shift-head">
    <span class="shift-title">
      {shift.title}
      {#if shift.outdoor}<span title="Outdoor shift">🌤</span>{/if}
    </span>
    <span class="chip chip-{shift.status}">{shift.status}</span>
  </div>
  <div class="shift-meta">
    <span>{shift.start}–{shift.end}</span>
    <span>·</span>
    <span>{shift.location}</span>
  </div>
  <div class="shift-assignee">
    {#if shift.assignee}
      <span class="assignee-name">👤 {shift.assignee}</span>
    {:else}
      <span class="muted">Unassigned</span>
    {/if}
  </div>
</article>
