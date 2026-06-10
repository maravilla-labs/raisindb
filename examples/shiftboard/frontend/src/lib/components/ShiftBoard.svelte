<script lang="ts">
  import { board, DAYS, type Shift } from '../stores/board.svelte';
  import ShiftCard from './ShiftCard.svelte';

  const DAY_LABELS: Record<Shift['day'], string> = {
    friday: 'Friday',
    saturday: 'Saturday',
    sunday: 'Sunday',
  };

  // Group shifts by day; reactive against the live-updating board store.
  const byDay = $derived(
    DAYS.map((day) => ({
      day,
      label: DAY_LABELS[day],
      shifts: board.shifts.filter((s) => s.day === day),
    })),
  );
</script>

<section class="panel board-panel">
  <h2 class="panel-title">Shift board</h2>

  {#if board.loading}
    <p class="muted">Loading board…</p>
  {:else if board.error}
    <p class="error-banner">{board.error}</p>
  {:else}
    {#each byDay as group (group.day)}
      <div class="day-group">
        <h3 class="day-title">{group.label}</h3>
        <div class="day-cards">
          {#each group.shifts as shift (shift.path)}
            <!-- Keying on flashSeq recreates the card on each live update,
                 which replays the highlight animation. -->
            {#key shift.flashSeq}
              <ShiftCard {shift} />
            {/key}
          {:else}
            <p class="muted">No shifts.</p>
          {/each}
        </div>
      </div>
    {/each}

    <h3 class="day-title staff-title">Staff</h3>
    <ul class="staff-list">
      {#each board.staff as member (member.path)}
        <li class="staff-row">
          <span class="staff-name">{member.title}</span>
          <span class="staff-role">{member.role}</span>
          <span class="staff-days">
            {#each member.availableDays as day (day)}
              <span class="chip chip-day">{day.slice(0, 3)}</span>
            {/each}
          </span>
        </li>
      {/each}
    </ul>
  {/if}
</section>
