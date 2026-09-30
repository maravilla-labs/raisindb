import fs from 'fs';

/** Rotate the log the server writes itself (`RAISIN_LOG_FILE`) at this size. */
export const SERVER_LOG_MAX_MB = 50;
/** Rotated server logs kept (`server.log.1` … `.N`). */
export const SERVER_LOG_MAX_FILES = 5;
/**
 * The console log only receives what bypasses the server's logger (early
 * start-up output, panics) — or everything, from a server binary too old to
 * know `RAISIN_LOG_FILE`. Either way it must not grow without bound, so it is
 * rotated at every start once it passes this size.
 */
export const CONSOLE_LOG_MAX_BYTES = 10 * 1024 * 1024;

/**
 * Rename `file` to `file.1` (shifting older copies, keeping `keep`) when it is
 * larger than `maxBytes`. Returns whether it rotated. Never throws: a log that
 * cannot be rotated must not stop the server from starting.
 */
export function rotateIfLarger(file: string, maxBytes: number, keep = 1): boolean {
  try {
    if (!fs.existsSync(file) || fs.statSync(file).size <= maxBytes) return false;
    if (keep <= 0) {
      fs.truncateSync(file, 0);
      return true;
    }
    fs.rmSync(`${file}.${keep}`, { force: true });
    for (let n = keep - 1; n >= 1; n--) {
      if (fs.existsSync(`${file}.${n}`)) fs.renameSync(`${file}.${n}`, `${file}.${n + 1}`);
    }
    fs.renameSync(file, `${file}.1`);
    return true;
  } catch {
    return false;
  }
}

/**
 * The log `raisindb server logs` should show: the server's own log, unless the
 * console log is newer (a server binary that predates `RAISIN_LOG_FILE` writes
 * everything to the console log).
 */
export function pickLogFile(serverLog: string, consoleLog: string): string | null {
  const mtime = (f: string) => (fs.existsSync(f) ? fs.statSync(f).mtimeMs : -1);
  const s = mtime(serverLog);
  const c = mtime(consoleLog);
  if (s < 0 && c < 0) return null;
  return c > s ? consoleLog : serverLog;
}
