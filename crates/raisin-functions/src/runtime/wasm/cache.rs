// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Content-addressed cache for compiled components.
//!
//! Cranelift takes seconds on a jco/StarlingMonkey component, so compiling on
//! every invocation is not a slow path, it is an unusable one. This module is
//! what makes a wasm function cheap after its first call.
//!
//! # The key is OUR hash of the bytes
//!
//! `blake3(artifact)`, never the node's `content_hash`. `raisin:Asset` is
//! `strict: false`, so a tenant can write any `content_hash` it likes — using
//! it as the cache key would let tenant A publish an artifact claiming tenant
//! B's hash and be handed B's compiled code. Hashing the bytes ourselves makes
//! that impossible: two entries collide only if they ARE the same artifact.
//!
//! It also means there is nothing to invalidate. An edited function is
//! different bytes, so it is a different key; the old entry ages out by weight.
//! Do **not** wire this to `FunctionModuleEventHandler` or the derived-cache
//! registry — a content-addressed cache has no staleness to be told about.
//!
//! # What is shared, and what that costs
//!
//! A cache entry is the ONE thing two tenants share when they run byte-identical
//! artifacts, and it holds no mutable state: an `InstancePre` is linked code,
//! and every execution still builds its own `Store`. Sharing it is what makes
//! the one-artifact-N-functions layout free.
//!
//! # Single flight
//!
//! Eight concurrent cold calls for the same artifact must compile it once, not
//! eight times — otherwise a burst on a fresh deploy burns eight cores for
//! seconds each. `KeyedMutex` QUEUES per key (unlike `raisin_locks`, which
//! rejects), so the losers wait and then take the cache hit the winner left.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};

use moka::sync::Cache;
use raisin_error::{Error, Result};
use raisin_rocksdb::KeyedMutex;
use wasmtime::component::InstancePre;

use super::bindings::HostState;
use super::compile::{compile_component, link};
use super::config::config;
use super::engine::engine;

/// One compiled, linked artifact, ready to instantiate into a fresh `Store`.
pub struct CachedComponent {
    /// Linked code. Cloning it is a refcount bump, not a compile.
    pub instance_pre: InstancePre<HostState>,
    /// Size of the compiled image, the unit the cache budget is spent in.
    pub image_bytes: u32,
    /// Size of the source artifact, for logging.
    pub artifact_bytes: usize,
}

impl std::fmt::Debug for CachedComponent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CachedComponent")
            .field("image_bytes", &self.image_bytes)
            .field("artifact_bytes", &self.artifact_bytes)
            .finish_non_exhaustive()
    }
}

/// Compiled artifacts, evicted LRU by compiled-image size.
///
/// The weigher runs exactly ONCE, at insert (moka has no re-weigh API), which
/// is correct here: a compiled image never changes size.
static COMPILED: LazyLock<Cache<String, Arc<CachedComponent>>> = LazyLock::new(|| {
    Cache::builder()
        .weigher(|_key: &String, value: &Arc<CachedComponent>| -> u32 { value.image_bytes })
        .max_capacity(config().compiled_cache_bytes)
        .build()
});

/// One in-flight compile per artifact.
static COMPILE_LOCKS: LazyLock<Arc<KeyedMutex<String>>> =
    LazyLock::new(|| Arc::new(KeyedMutex::new()));

/// Compiles since boot, across every artifact.
static COMPILES_TOTAL: AtomicU64 = AtomicU64::new(0);

/// Compiles per artifact, for tests and for spotting an artifact that keeps
/// being evicted and recompiled.
///
/// Bounded like everything else here: it is a moka cache with an entry-count
/// budget, so a long-lived server that has seen a hundred thousand artifact
/// versions does not keep a counter for each.
static COMPILES_BY_KEY: LazyLock<Cache<String, u64>> =
    LazyLock::new(|| Cache::builder().max_capacity(4096).build());

/// The cache key: blake3 of the artifact bytes, hex.
fn artifact_key(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// Refuse an artifact larger than the configured cap.
///
/// One implementation, called from the execution path, the cache and
/// `validate_component`, so an artifact cannot be accepted at upload and then
/// refused at run time (or the other way round).
pub fn check_artifact_cap(len: usize) -> Result<()> {
    crate::types::check_wasm_artifact_size(len, super::config::max_artifact_bytes())
}

/// Compiled-image size, as moka's `u32` weight.
///
/// `image_range` is a pointer range over code wasmtime already holds; reading
/// its length allocates nothing, where `Component::serialize()` would copy the
/// whole image just to measure it. Clamped rather than wrapped, and never zero
/// — a zero-weight entry is free and would never be evicted.
fn image_bytes(instance_pre: &InstancePre<HostState>) -> u32 {
    let range = instance_pre.component().image_range();
    let len = (range.end as usize).saturating_sub(range.start as usize);
    len.clamp(1, u32::MAX as usize) as u32
}

/// Fetch the compiled form of `bytes`, compiling it at most once.
///
/// Cranelift never runs on a tokio worker: the miss path hands it to
/// `spawn_blocking`. There is deliberately no `block_in_place` anywhere in this
/// module — it would pin a worker thread for the whole compile.
pub async fn get_or_compile(bytes: Arc<[u8]>) -> Result<Arc<CachedComponent>> {
    check_artifact_cap(bytes.len())?;
    let key = artifact_key(&bytes);

    if let Some(hit) = COMPILED.get(&key) {
        return Ok(hit);
    }

    // Cold. Queue behind any compile already in flight for this artifact, then
    // look again: the winner will have filled the cache while we waited.
    let _flight = COMPILE_LOCKS.lock(key.clone()).await;
    if let Some(hit) = COMPILED.get(&key) {
        return Ok(hit);
    }

    tokio::task::spawn_blocking(move || load_or_compile(key, &bytes))
        .await
        .map_err(|e| Error::Internal(format!("wasm compile task failed: {e}")))?
}

/// The miss path, on a blocking thread: the on-disk image if there is one,
/// otherwise Cranelift — and either way the result goes into the cache.
fn load_or_compile(key: String, bytes: &[u8]) -> Result<Arc<CachedComponent>> {
    let artifact_bytes = bytes.len();
    let started = std::time::Instant::now();

    let (component, source) = match disk::load(&key) {
        Some(component) => (component, "disk"),
        None => {
            let component = compile_component(bytes)?;
            COMPILES_TOTAL.fetch_add(1, Ordering::Relaxed);
            let previous = COMPILES_BY_KEY.get(&key).unwrap_or(0);
            COMPILES_BY_KEY.insert(key.clone(), previous + 1);
            disk::store(&key, &component);
            (component, "cranelift")
        }
    };
    let instance_pre = link(&component)?;

    let entry = Arc::new(CachedComponent {
        image_bytes: image_bytes(&instance_pre),
        instance_pre,
        artifact_bytes,
    });

    tracing::info!(
        artifact_key = %key,
        artifact_bytes,
        image_bytes = entry.image_bytes,
        source,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "wasm component ready"
    );

    COMPILED.insert(key, entry.clone());
    Ok(entry)
}

/// Decide whether an artifact could ever run on this host — and, since the
/// answer costs a full compile, keep what the compile produced.
///
/// The same compile the cache-miss path takes, so an artifact accepted at
/// upload cannot be rejected at run time. It used to throw the compiled code
/// away ("an artifact that is uploaded but never invoked should not hold
/// compiled-code budget"), which made every deploy pay Cranelift TWICE: once in
/// the installer, and again inside the first request to the function — the
/// 1–3.5 s cold start a site saw after each package install, long enough to
/// trip its client timeouts. The budget argument does not hold up: the cache
/// is LRU by weight, so an artifact nobody invokes is simply the first thing
/// evicted.
///
/// Used by the HTTP upload path and the package installer (through the
/// `raisin-rocksdb` validator inversion); a build without the `wasm` feature
/// gets the stub in `runtime::validate_component`, which warns and accepts.
pub fn validate_component(bytes: &[u8]) -> Result<()> {
    check_artifact_cap(bytes.len())?;
    let key = artifact_key(bytes);
    if COMPILED.contains_key(&key) {
        return Ok(());
    }
    load_or_compile(key, bytes).map(|_| ())
}

/// [`validate_component`] for an async caller.
///
/// Cranelift is seconds of CPU on a jco/StarlingMonkey artifact, so validating
/// one on a tokio worker parks that worker — a handful of concurrent uploads
/// would occupy the whole runtime and stall unrelated requests. Every async
/// path (the HTTP upload handler, the package installer's job worker) must go
/// through THIS, exactly as the cache-miss path does; the synchronous form
/// stays for callers that are already on a blocking thread.
pub async fn validate_component_async(bytes: Arc<[u8]>) -> Result<()> {
    get_or_compile(bytes).await.map(|_| ())
}

/// Compiled images on disk, so a restart does not recompile every function.
///
/// `<compiled_dir>/<blake3 of the artifact>-<engine fingerprint>.cwasm`. The
/// fingerprint is wasmtime's own compatibility hash (version, target, every
/// setting that changes code generation), so a different binary or a changed
/// `[functions.wasm]` looks for a different file instead of loading code built
/// for another engine; wasmtime re-checks the header on load regardless.
///
/// Loading an image is `unsafe` in wasmtime's terms because the bytes are
/// executed as native code. The directory lives inside the server's own data
/// directory and only this module writes it, atomically (temp file + rename),
/// so it is trusted exactly as far as the RocksDB files beside it are. Any
/// failure — unreadable, truncated, incompatible — is a miss, never an error:
/// the artifact is compiled from source as before.
mod disk {
    use std::hash::{Hash, Hasher};
    use std::path::PathBuf;
    use std::sync::LazyLock;

    use wasmtime::component::Component;

    use super::{config, engine};

    static ENGINE_FINGERPRINT: LazyLock<Option<String>> = LazyLock::new(|| {
        let engine = engine().ok()?;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        engine.precompile_compatibility_hash().hash(&mut hasher);
        Some(format!("{:016x}", hasher.finish()))
    });

    fn path(key: &str) -> Option<PathBuf> {
        let dir = config().compiled_dir.as_ref()?;
        let fingerprint = ENGINE_FINGERPRINT.as_ref()?;
        Some(dir.join(format!("{key}-{fingerprint}.cwasm")))
    }

    pub(super) fn load(key: &str) -> Option<Component> {
        let path = path(key)?;
        if !path.is_file() {
            return None;
        }
        // SAFETY: see the module docs — the file was written by `store` below
        // from `Component::serialize` of this same engine configuration, and
        // wasmtime validates the header and compatibility before mapping it.
        match unsafe { Component::deserialize_file(engine().ok()?, &path) } {
            Ok(component) => Some(component),
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "discarding unusable compiled wasm image");
                let _ = std::fs::remove_file(&path);
                None
            }
        }
    }

    pub(super) fn store(key: &str, component: &Component) {
        let Some(path) = path(key) else {
            return;
        };
        let written = (|| -> std::io::Result<()> {
            let image = component.serialize().map_err(std::io::Error::other)?;
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let tmp = path.with_extension(format!("tmp{}", std::process::id()));
            std::fs::write(&tmp, image)?;
            std::fs::rename(&tmp, &path)
        })();
        if let Err(e) = written {
            tracing::warn!(path = %path.display(), error = %e, "could not persist compiled wasm image");
        }
    }
}

/// Artifacts compiled since boot. Rises only on a cache miss.
pub fn compile_count() -> u64 {
    COMPILES_TOTAL.load(Ordering::Relaxed)
}

/// How many times this exact artifact has been compiled.
///
/// `1` after any number of executions is what a working cache looks like; a
/// number that keeps climbing means the artifact is being evicted between
/// calls and `compiled_cache_bytes` is too small.
pub fn compile_count_of(bytes: &[u8]) -> u64 {
    COMPILES_BY_KEY.get(&artifact_key(bytes)).unwrap_or(0)
}
