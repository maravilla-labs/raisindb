// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Crypto API bindings
//!
//! Provides UUID generation, JWT verify/sign, CSPRNG bytes, digests and ES256
//! key generation to both QuickJS (JavaScript) and Starlark (Python) runtimes.

use crate::api::FunctionApi;
use crate::runtime::bindings::registry::{
    ApiMethodDescriptor, ArgParser, ArgSpec, ArgType, InvokeResult, ReturnType,
};
use futures::future::BoxFuture;
use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

/// Get all crypto operation method descriptors
pub fn methods() -> Vec<ApiMethodDescriptor> {
    vec![
        // crypto.uuid() -> string (UUID v4)
        ApiMethodDescriptor {
            internal_name: "crypto_uuid",
            js_name: "uuid",
            py_name: "uuid",
            category: "crypto",
            args: vec![],
            return_type: ReturnType::String,
            invoker: |_api: Arc<dyn FunctionApi>,
                      _args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move { Ok(InvokeResult::String(uuid::Uuid::new_v4().to_string())) })
            },
        },
        // crypto.verifyJwt(token, opts?) -> { valid, claims?, error? }
        //   opts: { jwks_url?, issuer?, audience?, algorithms? }
        ApiMethodDescriptor {
            internal_name: "crypto_verify_jwt",
            js_name: "verifyJwt",
            py_name: "verify_jwt",
            category: "crypto",
            args: vec![
                ArgSpec::new("token", ArgType::String),
                ArgSpec::new("opts", ArgType::OptionalJson),
            ],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let token = parser.string()?;
                    let opts = parser.optional_json()?.unwrap_or(Value::Null);
                    let result = api.crypto_verify_jwt(&token, opts).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
        // crypto.randomBytes(n) -> base64 string of n CSPRNG bytes (1..=64)
        ApiMethodDescriptor {
            internal_name: "crypto_random_bytes",
            js_name: "randomBytes",
            py_name: "random_bytes",
            category: "crypto",
            args: vec![ArgSpec::new("n", ArgType::U32)],
            return_type: ReturnType::String,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let n = parser.u32()?;
                    Ok(InvokeResult::String(api.crypto_random_bytes(n).await?))
                })
            },
        },
        // crypto.hash(input, alg?) -> lowercase hex digest ("sha256" | "sha512")
        ApiMethodDescriptor {
            internal_name: "crypto_hash",
            js_name: "hash",
            py_name: "hash",
            category: "crypto",
            args: vec![
                ArgSpec::new("input", ArgType::String),
                ArgSpec::new("alg", ArgType::OptionalString),
            ],
            return_type: ReturnType::String,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let input = parser.string()?;
                    let alg = parser.optional_string()?;
                    let out = api.crypto_hash(&input, alg.as_deref()).await?;
                    Ok(InvokeResult::String(out))
                })
            },
        },
        // crypto.generateKeyPair(alg?) -> { alg, publicJwk, privateJwk }
        ApiMethodDescriptor {
            internal_name: "crypto_generate_key_pair",
            js_name: "generateKeyPair",
            py_name: "generate_key_pair",
            category: "crypto",
            args: vec![ArgSpec::new("alg", ArgType::OptionalString)],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let alg = parser.optional_string()?;
                    let out = api.crypto_generate_key_pair(alg.as_deref()).await?;
                    Ok(InvokeResult::Json(out))
                })
            },
        },
        // crypto.signJwt(claims, privateJwk, opts?) -> compact JWS string
        //   opts: { alg?, kid?, expiresInSec? }
        ApiMethodDescriptor {
            internal_name: "crypto_sign_jwt",
            js_name: "signJwt",
            py_name: "sign_jwt",
            category: "crypto",
            args: vec![
                ArgSpec::new("claims", ArgType::Json),
                ArgSpec::new("private_jwk", ArgType::Json),
                ArgSpec::new("opts", ArgType::OptionalJson),
            ],
            return_type: ReturnType::String,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let claims = parser.json()?;
                    let private_jwk = parser.json()?;
                    let opts = parser.optional_json()?.unwrap_or(Value::Null);
                    let out = api.crypto_sign_jwt(claims, private_jwk, opts).await?;
                    Ok(InvokeResult::String(out))
                })
            },
        },
    ]
}
