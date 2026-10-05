//! The average-rate cap of a paced build pass (`build::write_paced`).

/// Sleeps after each committed batch until the average write rate is at or
/// under `max_bytes_per_sec` (0: unlimited).
pub(super) struct Pace {
    max_bytes_per_sec: u64,
    started: std::time::Instant,
    written: u64,
}

impl Pace {
    pub(super) fn new(max_bytes_per_sec: u64) -> Self {
        Self {
            max_bytes_per_sec,
            started: std::time::Instant::now(),
            written: 0,
        }
    }

    pub(super) fn after_commit(&mut self, bytes: u64) {
        if self.max_bytes_per_sec == 0 {
            return;
        }
        self.written += bytes;
        let due =
            std::time::Duration::from_secs_f64(self.written as f64 / self.max_bytes_per_sec as f64);
        let elapsed = self.started.elapsed();
        if due > elapsed {
            std::thread::sleep(due - elapsed);
        }
    }
}
