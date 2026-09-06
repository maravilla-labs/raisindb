// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The process-wide wasmtime `Engine`, its epoch ticker and its `Linker`.
//!
//! All three are built once, lazily, from [`super::config::config()`]. The
//! engine owns compiled code and JIT mappings; a second engine would double
//! them and could not share a cached component, so there is exactly one.
//!
//! # The WASI subset is the egress boundary
//!
//! `io`, `clocks`, `random`, `cli` and `filesystem` are linked; `sockets` and
//! `http` are NOT, deliberately. Nothing enforces that later: an import the
//! linker does not carry fails at `instantiate_pre`, which is why an uploaded
//! component that reaches for `wasi:sockets` is rejected by name. Egress stays
//! `raisin.http.*`, where the tenant's network policy and the `EgressPolicy`
//! apply.
//!
//! `filesystem` is linked with ZERO preopens rather than omitted: wasi-libc
//! and StarlingMonkey call `filesystem/preopens` during startup even when the
//! guest never touches a file, so omitting it breaks every TinyGo and jco
//! component before `handler` is reached. With no preopens the guest sees an
//! empty root and every path is denied.

//! # An invalid configuration fails at BOOT, not at the first invocation
//!
//! Both statics hold a `Result`. `Engine::new` genuinely can fail — a pooling
//! reservation the host will not give, a stack size it refuses — and a
//! `LazyLock` initialiser that panics poisons the cell, so an `.expect()` here
//! would turn one bad `[functions.wasm]` value into a panic inside whichever
//! request happened to be first AND every wasm execution and every `.wasm`
//! upload for the life of the process. Storing the error instead makes it an
//! ordinary `Err` for every caller, and [`init`] — called from `main.rs` right
//! after the configuration is installed — forces the build so the operator
//! learns at startup, which is what the config docs promise.

use std::sync::LazyLock;

use raisin_error::{Error, Result};
use wasmtime::component::Linker;
use wasmtime::{Config, Engine, InstanceAllocationStrategy, PoolingAllocationConfig};
use wasmtime_wasi::cli::{WasiCli, WasiCliView};
use wasmtime_wasi::clocks::{WasiClocks, WasiClocksView};
use wasmtime_wasi::filesystem::{WasiFilesystem, WasiFilesystemView};
use wasmtime_wasi::p2::bindings::{cli, clocks, filesystem, io, random};
use wasmtime_wasi::random::{WasiRandom, WasiRandomView};
use wasmtime_wasi::WasiView;

use super::bindings::{host, HasHostState, HostState};
use super::config::{config, WasmAllocationStrategy};

/// The single engine. Building it also starts the epoch ticker.
static ENGINE: LazyLock<std::result::Result<Engine, String>> = LazyLock::new(build_engine);

/// The single linker. `Linker<T>` is generic over the store data, and every
/// wasm execution uses [`HostState`], so one is enough.
static LINKER: LazyLock<std::result::Result<Linker<HostState>, String>> =
    LazyLock::new(build_linker);

/// The process-wide engine, built on first use.
///
/// `Err` means `[functions.wasm]` names a configuration wasmtime refused; the
/// message is the same one [`init`] reports at boot.
pub fn engine() -> Result<&'static Engine> {
    ENGINE.as_ref().map_err(config_error)
}

/// The process-wide linker, carrying the host gateway and the WASI subset.
pub fn linker() -> Result<&'static Linker<HostState>> {
    LINKER.as_ref().map_err(config_error)
}

/// Build the engine and linker now, so a bad configuration is a boot failure.
///
/// Called from `main.rs` immediately after `configure_wasm_runtime`. A server
/// with `enabled = false` skips it: nothing can execute, and refusing to start
/// over tunables that will never be used would be gratuitous.
pub fn init() -> Result<()> {
    if !config().enabled {
        return Ok(());
    }
    engine()?;
    linker()?;
    Ok(())
}

/// The stored construction failure, as an error for the caller.
fn config_error(message: &String) -> Error {
    Error::Internal(format!(
        "the WebAssembly runtime could not be built from [functions.wasm]: {message}"
    ))
}

fn build_engine() -> std::result::Result<Engine, String> {
    let conf = config();
    let mut cfg = Config::new();

    // `max_wasm_stack_bytes` is operator input, so the fiber stack it implies
    // is computed with checked arithmetic: on a nonsense value the wrapping
    // form would silently ask for a tiny stack instead of failing.
    let async_stack_size = conf
        .max_wasm_stack_bytes
        .checked_mul(2)
        .and_then(|v| v.checked_add(1024 * 1024))
        .ok_or_else(|| {
            format!(
                "max_wasm_stack_bytes = {} is too large: the fiber stack it implies \
                 (2x + 1 MiB) does not fit in a usize",
                conf.max_wasm_stack_bytes
            )
        })?;

    // NOTE: `Config::async_support` is deprecated and a no-op in wasmtime 47 —
    // async is always available when the crate's `async` feature is on, which
    // it is. Do not re-add the call; it only produces a deprecation warning.
    cfg.wasm_component_model(true)
        // Epoch interruption, not fuel: `max_instructions` is not enforced by
        // QuickJS either, and the 100M default would cut off every function.
        .epoch_interruption(true)
        .consume_fuel(false)
        .max_wasm_stack(conf.max_wasm_stack_bytes)
        // Must exceed `max_wasm_stack`, or wasmtime refuses to build: the
        // fiber's stack has to hold the guest stack plus host frames.
        .async_stack_size(async_stack_size)
        .wasm_backtrace_max_frames(std::num::NonZeroUsize::new(32))
        .parallel_compilation(true);

    match conf.allocation {
        WasmAllocationStrategy::OnDemand => {
            cfg.allocation_strategy(InstanceAllocationStrategy::OnDemand);
        }
        WasmAllocationStrategy::Pooling => {
            let mut pool = PoolingAllocationConfig::default();
            pool.total_component_instances(conf.max_instances)
                .total_core_instances(conf.max_instances * 4)
                .total_memories(conf.max_instances * 4)
                .total_tables(conf.max_instances * 4)
                .total_stacks(conf.max_instances);
            cfg.allocation_strategy(InstanceAllocationStrategy::Pooling(pool));
        }
    }

    let engine = Engine::new(&cfg).map_err(|e| format!("{e:#}"))?;
    spawn_epoch_ticker(engine.clone(), conf.epoch_tick_ms);
    Ok(engine)
}

/// Start the daemon thread that advances the engine's epoch.
///
/// It runs for the life of the process — named, so it is identifiable in a
/// stack dump — and holds a clone of the engine, which keeps the engine alive
/// but costs nothing else. One tick is the resolution of every wasm timeout.
fn spawn_epoch_ticker(engine: Engine, tick_ms: u64) {
    let period = std::time::Duration::from_millis(tick_ms.max(1));
    let spawned = std::thread::Builder::new()
        .name("raisin-wasm-epoch".to_string())
        .spawn(move || loop {
            std::thread::sleep(period);
            engine.increment_epoch();
        });
    if let Err(e) = spawned {
        tracing::error!(
            error = %e,
            "failed to start the wasm epoch ticker; wasm timeouts will fall back to \
             the outer tokio timeout only"
        );
    }
}

fn build_linker() -> std::result::Result<Linker<HostState>, String> {
    let engine = ENGINE.as_ref().map_err(|e| e.clone())?;
    let mut linker = Linker::new(engine);
    add_host_and_wasi(&mut linker).map_err(|e| format!("linker construction failed: {e:#}"))?;
    Ok(linker)
}

/// Link the RaisinDB host interface plus the WASI subset.
///
/// Written out interface by interface rather than calling
/// `wasmtime_wasi::p2::add_to_linker_async`: that helper also links
/// `wasi:sockets`, which is the one thing this runtime must not provide.
fn add_host_and_wasi(linker: &mut Linker<HostState>) -> wasmtime::Result<()> {
    host::add_to_linker::<HostState, HasHostState>(linker, |state| state)?;

    // wasi:io — streams and pollables. Required by cli and filesystem.
    io::error::add_to_linker::<HostState, HasIo>(linker, |s| s.ctx().table)?;
    io::poll::add_to_linker::<HostState, HasIo>(linker, |s| s.ctx().table)?;
    io::streams::add_to_linker::<HostState, HasIo>(linker, |s| s.ctx().table)?;

    // wasi:clocks — `now`/`resolution`. Guest code measures elapsed time.
    clocks::wall_clock::add_to_linker::<HostState, WasiClocks>(linker, |s: &mut HostState| {
        s.clocks()
    })?;
    clocks::monotonic_clock::add_to_linker::<HostState, WasiClocks>(
        linker,
        |s: &mut HostState| s.clocks(),
    )?;

    // wasi:random — a real CSPRNG. Guest hash maps seed from it at startup.
    random::random::add_to_linker::<HostState, WasiRandom>(linker, |s: &mut HostState| s.random())?;
    random::insecure::add_to_linker::<HostState, WasiRandom>(linker, |s: &mut HostState| {
        s.random()
    })?;
    random::insecure_seed::add_to_linker::<HostState, WasiRandom>(linker, |s: &mut HostState| {
        s.random()
    })?;

    // wasi:cli — stdout/stderr become log lines; env and args are empty.
    cli::exit::add_to_linker::<HostState, WasiCli>(linker, |s: &mut HostState| s.cli())?;
    cli::environment::add_to_linker::<HostState, WasiCli>(linker, |s: &mut HostState| s.cli())?;
    cli::stdin::add_to_linker::<HostState, WasiCli>(linker, |s: &mut HostState| s.cli())?;
    cli::stdout::add_to_linker::<HostState, WasiCli>(linker, |s: &mut HostState| s.cli())?;
    cli::stderr::add_to_linker::<HostState, WasiCli>(linker, |s: &mut HostState| s.cli())?;
    cli::terminal_input::add_to_linker::<HostState, WasiCli>(linker, |s: &mut HostState| s.cli())?;
    cli::terminal_output::add_to_linker::<HostState, WasiCli>(linker, |s: &mut HostState| s.cli())?;
    cli::terminal_stdin::add_to_linker::<HostState, WasiCli>(linker, |s: &mut HostState| s.cli())?;
    cli::terminal_stdout::add_to_linker::<HostState, WasiCli>(linker, |s: &mut HostState| s.cli())?;
    cli::terminal_stderr::add_to_linker::<HostState, WasiCli>(linker, |s: &mut HostState| s.cli())?;

    // wasi:filesystem — linked with zero preopens. See the module docs.
    filesystem::types::add_to_linker::<HostState, WasiFilesystem>(linker, |s: &mut HostState| {
        s.filesystem()
    })?;
    filesystem::preopens::add_to_linker::<HostState, WasiFilesystem>(
        linker,
        |s: &mut HostState| s.filesystem(),
    )?;

    // Deliberately NOT linked: wasi:sockets, wasi:http.
    Ok(())
}

/// `HasData` marker for the `wasi:io` interfaces, whose host state is the
/// resource table itself. `wasmtime_wasi` keeps its own copy private.
struct HasIo;

impl wasmtime::component::HasData for HasIo {
    type Data<'a> = &'a mut wasmtime::component::ResourceTable;
}
