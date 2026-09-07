//! Digest and encoding kernels: MD5, SHA256, TO_HEX.

use super::args::{arity, int, text};
use super::{Kernel, KernelCategory, KernelResult};
use crate::analyzer::Literal;
use sha2::Digest;

macro_rules! kernel {
    ($name:expr, $sig:expr, $f:expr) => {
        Kernel {
            name: $name,
            aliases: &[],
            category: KernelCategory::String,
            signature: $sig,
            deterministic: true,
            strict: true,
            func: $f,
        }
    };
}

pub(super) static KERNELS: &[Kernel] = &[
    kernel!("MD5", "MD5(text) -> TEXT", md5_fn),
    kernel!("SHA256", "SHA256(text) -> TEXT", sha256),
    kernel!("TO_HEX", "TO_HEX(integer) -> TEXT", to_hex),
];

fn md5_fn(args: &[Literal]) -> KernelResult {
    arity("MD5", args, 1, 1)?;
    let digest = md5::compute(text("MD5", args, 0)?.as_bytes());
    Ok(Literal::Text(format!("{:x}", digest)))
}

/// Hex-encoded, like `encode(sha256(x::bytea), 'hex')` in PostgreSQL.
fn sha256(args: &[Literal]) -> KernelResult {
    arity("SHA256", args, 1, 1)?;
    let digest = sha2::Sha256::digest(text("SHA256", args, 0)?.as_bytes());
    Ok(Literal::Text(hex::encode(digest)))
}

fn to_hex(args: &[Literal]) -> KernelResult {
    arity("TO_HEX", args, 1, 1)?;
    Ok(Literal::Text(format!("{:x}", int("TO_HEX", args, 0)?)))
}
