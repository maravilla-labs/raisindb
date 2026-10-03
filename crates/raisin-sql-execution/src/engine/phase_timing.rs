//! Per-statement phase timings, behind `RAISIN_SQL_PHASE_TIMING`.
//!
//! Answers "where does a sub-millisecond query spend its time" — parse and
//! semantic analysis, subquery binding, planning, execution — without a
//! profiler. Unset (the default) it costs one cached flag check per statement:
//! no clock reads, no stream wrapper.
//!
//! Set to any value other than `0`/`false`, each statement logs ONE line at
//! INFO on the `raisin_sql::phase_timing` target when its row stream is
//! drained, e.g.
//!
//! ```text
//! analyze_us=41 bind_us=0 plan_us=212 open_us=9 drain_us=388 rows=1 sql="SELECT ..."
//! ```
//!
//! `analyze` is parse + semantic analysis (the analyzer does both in one call),
//! `open` is building the operator tree, `drain` is pulling every row.

use crate::physical_plan::executor::RowStream;
use futures::StreamExt;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Whether timings are on for this process. Read once.
pub(crate) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("RAISIN_SQL_PHASE_TIMING")
            .map(|v| !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("false"))
            .unwrap_or(false)
    })
}

/// Phase marks for one statement. Inert when timings are off.
pub(crate) struct PhaseTimer {
    last: Option<Instant>,
    analyze: Duration,
    bind: Duration,
    plan: Duration,
    open: Duration,
}

impl PhaseTimer {
    pub(crate) fn start() -> Self {
        Self {
            last: enabled().then(Instant::now),
            analyze: Duration::ZERO,
            bind: Duration::ZERO,
            plan: Duration::ZERO,
            open: Duration::ZERO,
        }
    }

    /// Time since the previous mark, restarting the clock. Zero when off.
    fn lap(&mut self) -> Duration {
        match self.last {
            Some(last) => {
                let now = Instant::now();
                self.last = Some(now);
                now - last
            }
            None => Duration::ZERO,
        }
    }

    pub(crate) fn analyzed(&mut self) {
        self.analyze = self.lap();
    }

    pub(crate) fn bound(&mut self) {
        self.bind = self.lap();
    }

    pub(crate) fn planned(&mut self) {
        self.plan = self.lap();
    }

    /// Mark the stream open and, when timings are on, wrap it so the drain
    /// time and the line are recorded when the last row has been pulled.
    pub(crate) fn opened(mut self, stream: RowStream, sql: &str) -> RowStream {
        self.open = self.lap();
        let Some(drain_start) = self.last else {
            return stream;
        };
        let sql: String = sql.chars().take(200).collect();
        let (analyze, bind, plan, open) = (self.analyze, self.bind, self.plan, self.open);
        Box::pin(async_stream::stream! {
            let mut stream = stream;
            let mut rows = 0usize;
            while let Some(item) = stream.next().await {
                rows += 1;
                yield item;
            }
            tracing::info!(
                target: "raisin_sql::phase_timing",
                analyze_us = analyze.as_micros() as u64,
                bind_us = bind.as_micros() as u64,
                plan_us = plan.as_micros() as u64,
                open_us = open.as_micros() as u64,
                drain_us = drain_start.elapsed().as_micros() as u64,
                rows,
                sql = %sql,
                "sql phase timing"
            );
        })
    }
}
