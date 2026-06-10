<script lang="ts">
  import type { InboxTask, TaskOption } from '@raisindb/client';
  import { tasks } from '../stores/tasks.svelte';

  let panel = $state<HTMLElement>();
  let flashing = $state(false);

  // Tasks without an options array (review/action types) still need a way
  // to be completed — one generic acknowledge button.
  const FALLBACK_OPTIONS: TaskOption[] = [{ value: 'done', label: 'Mark done', style: 'success' }];

  function optionsFor(task: InboxTask): TaskOption[] {
    return task.options && task.options.length > 0 ? task.options : FALLBACK_OPTIONS;
  }

  function dueLabel(dueAt: string): string {
    const due = new Date(dueAt);
    if (Number.isNaN(due.getTime())) return '';
    const hours = (due.getTime() - Date.now()) / 36e5;
    if (hours < 0) return 'overdue';
    if (hours < 24) return `due in ${Math.max(1, Math.round(hours))}h`;
    return `due ${due.toLocaleDateString()}`;
  }

  function isOverdue(dueAt: string): boolean {
    const t = new Date(dueAt).getTime();
    return !Number.isNaN(t) && t < Date.now();
  }

  // Bell clicked: bring the panel into view and flash it briefly.
  $effect(() => {
    if (tasks.focusSeq === 0) return;
    panel?.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
    panel?.focus();
    flashing = true;
    const timer = setTimeout(() => (flashing = false), 1400);
    return () => clearTimeout(timer);
  });
</script>

<!-- Human-in-the-loop tasks are plain nodes the logged-in user can read;
     the panel only exists while there is something to decide (or an error
     to acknowledge). Buttons are driven entirely by each task's `options`
     array — nothing here is shift-specific. -->
{#if tasks.tasks.length > 0 || tasks.error}
  <section
    class="panel task-panel"
    class:flash={flashing}
    bind:this={panel}
    tabindex="-1"
    aria-label="Inbox tasks"
  >
    <h2 class="panel-title">
      Your tasks
      {#if tasks.tasks.length > 0}
        <span class="task-count">{tasks.tasks.length}</span>
      {/if}
    </h2>

    {#if tasks.error}
      <p class="error-banner" role="alert">{tasks.error}</p>
    {/if}

    {#if tasks.tasks.length === 0}
      <p class="muted">No pending tasks.</p>
    {:else}
      <ul class="task-list">
        {#each tasks.tasks as task (task.path)}
          <li class="task-card">
            <div class="task-head">
              <span class="task-title">{task.title}</span>
              <span class="task-chips">
                {#if task.priority != null}
                  <span class="chip chip-priority" class:high={task.priority >= 4}>
                    P{task.priority}
                  </span>
                {/if}
                {#if task.due_at}
                  <span class="chip chip-due" class:overdue={isOverdue(task.due_at)}>
                    {dueLabel(task.due_at)}
                  </span>
                {/if}
              </span>
            </div>
            {#if task.description}
              <p class="task-desc">{task.description}</p>
            {/if}
            <div class="task-actions">
              {#each optionsFor(task) as option (option.value)}
                <button
                  class="task-btn {option.style ?? 'neutral'}"
                  onclick={() => tasks.complete(task.id, option.value)}
                >
                  {option.label}
                </button>
              {/each}
            </div>
          </li>
        {/each}
      </ul>
    {/if}
  </section>
{/if}
