/**
 * Pure data mapping for the shift board (no Vue, no browser APIs).
 * Mirrors the shiftboard demo's board-data module — same node shapes,
 * same SQL — so both examples render the same server-side content.
 */

export interface Shift {
  path: string;
  title: string;
  day: 'friday' | 'saturday' | 'sunday';
  start: string;
  end: string;
  location: string;
  outdoor: boolean;
  status: 'open' | 'filled';
  assignee: string | null;
  /** Bumped on every live update; the UI keys on it to replay the flash animation. */
  flashSeq: number;
}

export interface StaffMember {
  path: string;
  title: string;
  role: string;
  availableDays: string[];
}

export const DAYS: Shift['day'][] = ['friday', 'saturday', 'sunday'];

/** Node properties as stored on /shifts/* nodes. */
export interface ShiftProps {
  title?: string;
  day?: string;
  start?: string;
  end?: string;
  location?: string;
  outdoor?: boolean;
  status?: string;
  assignee?: string | null;
}

/** Row shape of `SELECT path, properties FROM ...`. */
export interface SqlNodeRow {
  path: string;
  properties: Record<string, unknown>;
}

export function toShift(path: string, props: ShiftProps, flashSeq = 0): Shift {
  return {
    path,
    title: props.title ?? path.split('/').pop() ?? '',
    day: (props.day as Shift['day']) ?? 'friday',
    start: props.start ?? '',
    end: props.end ?? '',
    location: props.location ?? '',
    outdoor: props.outdoor === true,
    status: props.status === 'filled' ? 'filled' : 'open',
    assignee: props.assignee ?? null,
    flashSeq,
  };
}

export function rowsToShifts(rows: SqlNodeRow[]): Shift[] {
  return rows
    .map((row) => toShift(row.path, row.properties as ShiftProps))
    .sort(
      (a, b) =>
        DAYS.indexOf(a.day) - DAYS.indexOf(b.day) || a.start.localeCompare(b.start),
    );
}

export function rowsToStaff(rows: SqlNodeRow[]): StaffMember[] {
  return rows
    .map((row) => ({
      path: row.path,
      title: (row.properties.title as string) ?? row.path.split('/').pop() ?? '',
      role: (row.properties.role as string) ?? '',
      availableDays: (row.properties.available_days as string[]) ?? [],
    }))
    .sort((a, b) => a.title.localeCompare(b.title));
}

export const SHIFTS_SQL = "SELECT path, properties FROM 'staffing' WHERE CHILD_OF('/shifts')";
export const STAFF_SQL = "SELECT path, properties FROM 'staffing' WHERE CHILD_OF('/staff')";
