//! Tests for the one classifier.
//!
//! These build a model the way the discovery path does — capabilities plus a
//! metadata blob — and assert what a tenant's `AIModelConfig` would end up
//! offering. The load-bearing case is the second one: an embedding model whose
//! width the gateway did not publish must come out UNUSABLE, not guessed.

use serde_json::json;

use super::*;

fn ctx() -> ClassificationContext<'static> {
    ClassificationContext {
        tenant_id: Some("acme"),
        provider_slug: Some("maravilla"),
    }
}

/// A model as the Groq/custom client emits it once `apply_declared_kind` has
/// folded the gateway's two extension keys in.
fn from_gateway(id: &str, kind: Option<&str>, dimensions: Option<u64>) -> ModelInfo {
    let mut capabilities = ModelCapabilities {
        chat: true,
        embeddings: false,
        vision: false,
        tools: true,
        streaming: true,
    };
    let mut metadata = json!({ "owned_by": "maravilla", "created": 0, "active": true });
    apply_declared_kind(id, kind, dimensions, &mut capabilities, &mut metadata);
    ModelInfo::new(id, id)
        .with_capabilities(capabilities)
        .with_metadata(metadata)
}

#[test]
fn embedding_kind_with_width_is_offered() {
    let model = from_gateway(
        "maravilla/embed-multilingual",
        Some("embedding"),
        Some(3584),
    );
    let out = classify(&model, ctx());

    assert_eq!(out.use_cases, vec![AIUseCase::Embedding]);
    let meta = out.metadata.expect("metadata");
    assert_eq!(meta[METADATA_EMBEDDING_LENGTH], json!(3584));
    assert_eq!(meta[METADATA_KIND], json!("embedding"));
    assert!(meta.get(METADATA_EMBEDDING_UNAVAILABLE_REASON).is_none());
}

#[test]
fn embedding_kind_without_width_is_not_offered() {
    let model = from_gateway("maravilla/embed-broken", Some("embedding"), None);
    let out = classify(&model, ctx());

    // Not offered for anything: not as an embedder (no width), and NOT as a
    // chat model either — the "default to chat" fallback is skipped whenever
    // the gateway declared a kind.
    assert!(
        out.use_cases.is_empty(),
        "widthless embedder must be offered for nothing, got {:?}",
        out.use_cases
    );

    let meta = out.metadata.expect("metadata");
    // No width at all — not 0, not null, not a guess.
    assert!(
        meta.get(METADATA_EMBEDDING_LENGTH).is_none(),
        "no width may be published for a model that did not declare one"
    );
    assert_eq!(
        meta[METADATA_EMBEDDING_UNAVAILABLE_REASON],
        json!(EMBEDDING_UNAVAILABLE_REASON)
    );
    // Still listed, so the failure is diagnosable rather than a missing model.
    assert_eq!(model.id, "maravilla/embed-broken");
}

#[test]
fn an_implausible_width_is_treated_as_missing() {
    for bogus in [0u64, MAX_EMBEDDING_DIMENSIONS + 1] {
        let model = from_gateway("maravilla/embed-odd", Some("embedding"), Some(bogus));
        let out = classify(&model, ctx());
        assert!(
            out.use_cases.is_empty(),
            "width {bogus} must not be trusted"
        );
        assert!(out
            .metadata
            .expect("metadata")
            .get(METADATA_EMBEDDING_LENGTH)
            .is_none());
    }
}

#[test]
fn chat_kind_never_gets_the_embedding_use_case() {
    // Even with a stray `dimensions`, which the gateway's own loader rejects.
    let model = from_gateway("maravilla/balanced", Some("chat"), Some(3584));
    let out = classify(&model, ctx());

    assert!(!out.use_cases.contains(&AIUseCase::Embedding));
    assert_eq!(
        out.use_cases,
        vec![AIUseCase::Chat, AIUseCase::Completion, AIUseCase::Agent]
    );
    let meta = out.metadata.expect("metadata");
    assert!(meta.get(METADATA_EMBEDDING_LENGTH).is_none());
    assert_eq!(meta[METADATA_KIND], json!("chat"));
}

#[test]
fn an_unknown_kind_falls_back_to_the_heuristics() {
    // A future `rerank` must not error, panic, or drop the model: the
    // provider client's own guesses stand and the value is echoed for
    // diagnosis.
    let model = from_gateway("maravilla/rerank", Some("rerank"), None);
    let out = classify(&model, ctx());

    assert_eq!(
        out.use_cases,
        vec![AIUseCase::Chat, AIUseCase::Completion, AIUseCase::Agent]
    );
    assert_eq!(
        out.metadata.expect("metadata")[METADATA_KIND],
        json!("rerank")
    );
}

#[test]
fn no_kind_at_all_is_todays_behaviour() {
    // Real Groq, and any gateway predating the contract.
    let model = from_gateway("llama-3.3-70b-versatile", None, None);
    let out = classify(&model, ctx());

    assert_eq!(
        out.use_cases,
        vec![AIUseCase::Chat, AIUseCase::Completion, AIUseCase::Agent]
    );
    let meta = out.metadata.expect("metadata");
    assert!(meta.get(METADATA_KIND).is_none(), "nothing invented");
    assert!(meta.get(METADATA_EMBEDDING_LENGTH).is_none());
}

#[test]
fn a_heuristic_embedder_still_needs_a_width() {
    // The OpenAI and Ollama paths set `embeddings: true` from their own
    // heuristics with no `kind` key. They may only be offered when they also
    // published a width.
    let with_width = ModelInfo::new("text-embedding-3-small", "text-embedding-3-small")
        .with_capabilities(ModelCapabilities::embeddings())
        .with_metadata(json!({ "embedding_length": 1536 }));
    assert_eq!(
        classify(&with_width, ctx()).use_cases,
        vec![AIUseCase::Embedding]
    );

    let without_width = ModelInfo::new("mystery-embed", "mystery-embed")
        .with_capabilities(ModelCapabilities::embeddings())
        .with_metadata(json!({ "architecture": "mystery" }));
    let out = classify(&without_width, ctx());
    assert!(
        out.use_cases.is_empty(),
        "an embedder with no width is never silently downgraded to a chat model"
    );
    assert_eq!(
        out.metadata.expect("metadata")[METADATA_EMBEDDING_UNAVAILABLE_REASON],
        json!(EMBEDDING_UNAVAILABLE_REASON)
    );
}

#[test]
fn a_width_of_the_wrong_json_type_is_an_absence_not_an_error() {
    let model = ModelInfo::new("odd", "odd")
        .with_capabilities(ModelCapabilities::embeddings())
        .with_metadata(json!({ "kind": "embedding", "embedding_length": "3584" }));
    let out = classify(&model, ctx());

    assert!(out.use_cases.is_empty());
    // The untrusted value is stripped rather than left for the console to
    // auto-set `dimensions` from.
    assert!(out
        .metadata
        .expect("metadata")
        .get(METADATA_EMBEDDING_LENGTH)
        .is_none());
}

#[test]
fn declared_kind_reads_every_shape() {
    assert_eq!(declared_kind(None), DeclaredKind::Absent);
    assert_eq!(declared_kind(Some(&json!({}))), DeclaredKind::Absent);
    assert_eq!(
        declared_kind(Some(&json!({ "kind": null }))),
        DeclaredKind::Absent
    );
    assert_eq!(
        declared_kind(Some(&json!({ "kind": 7 }))),
        DeclaredKind::Absent
    );
    assert_eq!(
        declared_kind(Some(&json!({ "kind": "chat" }))),
        DeclaredKind::Chat
    );
    assert_eq!(
        declared_kind(Some(&json!({ "kind": "embedding" }))),
        DeclaredKind::Embedding
    );
    assert_eq!(
        declared_kind(Some(&json!({ "kind": "rerank" }))),
        DeclaredKind::Unknown
    );
}
