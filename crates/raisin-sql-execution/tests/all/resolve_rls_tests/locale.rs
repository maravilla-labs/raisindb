//! A target hidden in the row's locale is not part of the document in that
//! language. RESOLVE leaves it bare — exactly as it leaves a missing target —
//! rather than falling back to the base-language content the author withheld.

use super::*;
use raisin_context::RepositoryConfig;
use raisin_models::translations::{LocaleCode, LocaleOverlay, TranslationMeta};
use raisin_storage::TranslationRepository;

fn localized_engine(storage: &Arc<Store>) -> QueryEngine<Store> {
    let config = RepositoryConfig {
        default_language: "en".to_string(),
        supported_languages: vec!["en".to_string(), "de".to_string()],
        ..Default::default()
    };
    engine(storage, AuthContext::system()).with_repository_config(config)
}

async fn hide(storage: &Arc<Store>, workspace: &str, node_id: &str, locale: &str) {
    let locale = LocaleCode::parse(locale).expect("locale");
    let meta = TranslationMeta {
        locale: locale.clone(),
        revision: raisin_hlc::HLC::now(),
        parent_revision: None,
        timestamp: chrono::Utc::now(),
        actor: "test".to_string(),
        message: "hide".to_string(),
        is_system: true,
    };
    storage
        .translations()
        .store_translation(
            TENANT,
            REPO,
            BRANCH,
            workspace,
            node_id,
            &locale,
            &LocaleOverlay::Hidden,
            &meta,
        )
        .await
        .expect("hide");
}

#[tokio::test]
async fn resolve_hidden_in_locale_is_bare_ref() {
    let (storage, _dir) = setup().await;
    let sys = engine(&storage, AuthContext::system());
    insert(&sys, ASSETS, "shown", "/shown", json!({ "alt": "visible" })).await;
    insert(
        &sys,
        ASSETS,
        "hidden",
        "/hidden",
        json!({ "alt": "english only" }),
    )
    .await;
    insert(
        &sys,
        PAGES,
        "home",
        "/home",
        json!({
            "shown": reference("shown", ASSETS),
            "hidden": reference("hidden", ASSETS),
            "missing": reference("nope", ASSETS),
        }),
    )
    .await;
    hide(&storage, ASSETS, "hidden", "de").await;

    let e = localized_engine(&storage);
    let read = |locale: &str| {
        format!(
            "SELECT RESOLVE(properties) AS r, properties AS p FROM '{PAGES}' \
             WHERE path = '/home' AND locale = '{locale}'"
        )
    };

    // In the base language the node is part of the document.
    let en = rows(&e, &read("en")).await;
    assert_eq!(en[0]["r"]["hidden"]["alt"], "english only");

    // In German it is not: the reference stays exactly as stored, the same as
    // the reference to a node that does not exist.
    let de = rows(&e, &read("de")).await;
    let (r, p) = (&de[0]["r"], &de[0]["p"]);
    assert!(
        r["hidden"].get("alt").is_none(),
        "hidden target inlined: {r}"
    );
    assert_eq!(r["hidden"], p["hidden"]);
    assert_eq!(r["missing"], p["missing"]);
    assert_eq!(r["shown"]["alt"], "visible");
}
