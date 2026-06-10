<script setup lang="ts">
/**
 * Shift board: initial load via useSql, live updates via useSubscription.
 *
 * Shifts are mirrored into a local ref so live node:updated events can patch
 * a single card in place (bumping flashSeq replays the highlight animation
 * thanks to the :key on the card). Staff render straight from useSql data.
 */
import { computed, ref, watch } from 'vue';
import type { EventMessage } from '@raisindb/client';
import { db, useSql, useSubscription } from '../lib/raisin';
import {
  DAYS,
  SHIFTS_SQL,
  STAFF_SQL,
  rowsToShifts,
  rowsToStaff,
  toShift,
  type Shift,
  type ShiftProps,
  type SqlNodeRow,
} from '../lib/board-data';

const shiftsQuery = useSql<SqlNodeRow>(db, SHIFTS_SQL);
const staffQuery = useSql<SqlNodeRow>(db, STAFF_SQL);

const shifts = ref<Shift[]>([]);
// Note: the SDK's VueComputedRef is a structural type, not Vue's branded
// ComputedRef, so it can't be passed to watch() directly — use a getter.
watch(
  () => shiftsQuery.data.value,
  (rows) => {
    if (rows) shifts.value = rowsToShifts(rows);
  },
  { immediate: true },
);

const staff = computed(() => rowsToStaff(staffQuery.data.value ?? []));

const byDay = computed(() =>
  DAYS.map((day) => ({ day, cards: shifts.value.filter((s) => s.day === day) })),
);

const isLoading = computed(() => shiftsQuery.isLoading.value && shifts.value.length === 0);
const error = computed(() => shiftsQuery.error.value ?? staffQuery.error.value);

// Live updates: any node:updated under /shifts patches the matching card.
// `*` matches one path segment; include_node delivers the full node in the
// payload so no follow-up query is needed.
useSubscription(
  db,
  {
    workspace: 'staffing',
    path: '/shifts/*',
    event_types: ['node:updated'],
    include_node: true,
  },
  (event: EventMessage) => {
    const payload = event.payload as
      | { node?: { path?: string; properties?: ShiftProps } }
      | undefined;
    const node = payload?.node;
    if (!node?.path || !node.properties) return;

    const idx = shifts.value.findIndex((s) => s.path === node.path);
    if (idx < 0) return;
    shifts.value[idx] = toShift(node.path, node.properties, shifts.value[idx].flashSeq + 1);
  },
);
</script>

<template>
  <section class="panel board-panel" data-testid="board">
    <h2 class="panel-title">Weekend board</h2>

    <p v-if="error" class="error-banner" role="alert">{{ error.message }}</p>
    <p v-else-if="isLoading" class="muted">Loading shifts…</p>

    <div v-for="group in byDay" :key="group.day" class="day-group">
      <h3 class="day-title">{{ group.day }}</h3>
      <div class="day-cards">
        <!-- key includes flashSeq so a live update re-mounts the card and
             replays the flash animation -->
        <article
          v-for="shift in group.cards"
          :key="`${shift.path}:${shift.flashSeq}`"
          class="shift-card"
          :class="{ flash: shift.flashSeq > 0 }"
          :data-path="shift.path"
          data-testid="shift-card"
        >
          <div class="shift-head">
            <span class="shift-title">
              {{ shift.title }}
              <span v-if="shift.outdoor" title="Outdoor shift">🌤</span>
            </span>
            <span class="chip" :class="`chip-${shift.status}`">{{ shift.status }}</span>
          </div>
          <div class="shift-meta">
            <span>{{ shift.start }}–{{ shift.end }}</span>
            <span>·</span>
            <span>{{ shift.location }}</span>
          </div>
          <div class="shift-assignee">
            <span v-if="shift.assignee" class="assignee-name">👤 {{ shift.assignee }}</span>
            <span v-else class="muted">Unassigned</span>
          </div>
        </article>
      </div>
    </div>

    <h2 class="panel-title staff-title">Staff</h2>
    <ul class="staff-list" data-testid="staff-list">
      <li v-for="member in staff" :key="member.path" class="staff-row">
        <span class="staff-name">{{ member.title }}</span>
        <span class="staff-role">{{ member.role }}</span>
        <span>
          <span v-for="day in member.availableDays" :key="day" class="chip chip-day">{{ day }}</span>
        </span>
      </li>
    </ul>
  </section>
</template>
