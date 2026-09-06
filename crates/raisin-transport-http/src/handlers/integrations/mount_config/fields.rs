// SPDX-License-Identifier: BSL-1.1

//! Which `sync_config` keys this endpoint may write, and what a valid value for
//! each one looks like.
//!
//! The table is an ALLOW-LIST rather than a deny-list, and that direction is the
//! whole safety property: a mount node carries `state` (the delta cursor, the
//! push subscription id, the backfill resume point), `integration_ref`,
//! `account_ref` and `remote_root` beside its config, and a patch that could
//! name any of them would be the generic node API with extra steps. A key that
//! is not here cannot be written, whatever it is called.
//!
//! Keys the engine reads but that are deliberately NOT writable get an entry in
//! [`REFUSED`] instead, so an operator asking for one is told WHY rather than
//! "unknown field" — the difference between a setting that lives somewhere else
//! and a typo.

use serde_json::Value;

/// What shape a writable key accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Bool,
    /// A count. Bounds are per-field so a nonsense value is refused at the door
    /// rather than becoming a mount that hammers a provider every second.
    Count {
        min: u64,
        max: u64,
    },
    /// A count that may also be `null`, which CLEARS it. Only the two TTLs are
    /// nullable: for them "unset" is a distinct, meaningful state, while a
    /// cleared bool or a cleared interval is just the default written out.
    NullableCount {
        min: u64,
        max: u64,
    },
    Text,
    TextList,
    /// `poll` | `webhook` | `hybrid`.
    Mode,
}

/// One writable `sync_config` key.
pub(crate) struct Field {
    pub name: &'static str,
    pub kind: Kind,
}

/// A day. An interval longer than this is almost certainly a units mistake
/// (milliseconds pasted into a seconds field), and a mount that syncs once a
/// week reads as broken long before anyone checks the config.
const MAX_INTERVAL_SECS: u64 = 86_400;
/// Floor for the poll interval. Below this the scheduler tick (60s) is the real
/// limit anyway, so a smaller number buys nothing and only invites a mount that
/// is due on every tick against a provider's rate limit.
const MIN_INTERVAL_SECS: u64 = 30;

/// Every key this endpoint may write.
///
/// Deliberately absent: `batch_size` and `batch_max_bytes`. They are a PAIR —
/// one batch commits as a single replication record, and the byte budget is what
/// keeps a large batch under the 10 MB transport frame cap — so raising one
/// alone can wedge a peer's sync permanently. A control that is only safe when
/// two numbers move together does not belong on a per-field patch.
pub(crate) const WRITABLE: &[Field] = &[
    Field {
        name: "mode",
        kind: Kind::Mode,
    },
    Field {
        name: "interval_seconds",
        kind: Kind::Count {
            min: MIN_INTERVAL_SECS,
            max: MAX_INTERVAL_SECS,
        },
    },
    Field {
        name: "ephemeral",
        kind: Kind::Bool,
    },
    Field {
        name: "ttl_seconds",
        kind: Kind::NullableCount {
            min: 60,
            max: 10 * 365 * 86_400,
        },
    },
    Field {
        name: "cache_content",
        kind: Kind::Bool,
    },
    Field {
        name: "content_ttl_seconds",
        // Zero is meaningful here and nowhere else: it drops the bytes as soon
        // as the jobs that follow a fetch are done, which is what a large drive
        // wants.
        kind: Kind::NullableCount {
            min: 0,
            max: 10 * 365 * 86_400,
        },
    },
    Field {
        name: "max_items_per_sync",
        kind: Kind::Count {
            min: 1,
            max: 50_000,
        },
    },
    Field {
        name: "max_item_failures",
        kind: Kind::Count {
            min: 1,
            max: 10_000,
        },
    },
    Field {
        name: "path_template",
        kind: Kind::Text,
    },
    Field {
        name: "include_patterns",
        kind: Kind::TextList,
    },
    Field {
        name: "exclude_patterns",
        kind: Kind::TextList,
    },
    Field {
        name: "folder_node_types",
        kind: Kind::TextList,
    },
    Field {
        name: "allow_empty_reconcile",
        kind: Kind::Bool,
    },
    Field {
        name: "reconcile_deletes",
        kind: Kind::Bool,
    },
];

/// Keys an operator will reasonably reach for and which this endpoint refuses on
/// purpose, each with the reason it is refused.
///
/// Saying "unknown field `accepts_content`" would reproduce the exact failure
/// this endpoint exists to remove: a setting whose absence is indistinguishable
/// from a setting that does not work.
pub(crate) const REFUSED: &[(&str, &str)] = &[
    (
        "accepts_content",
        "`accepts_content` is an ADAPTER capability, not a mount setting: whether a \
         provider's objects have bytes at all is a fact about the provider. The \
         connector declares it per resource (the ms-graph connector declares it for \
         `resource: \"files\"` and not for mail or calendar), and the engine reads it \
         from the connector's cached capabilities on every run. To get it, point the \
         mount at a resource whose adapter declares it — `sync_config.resource` in the \
         mount editor — then re-run Test connection",
    ),
    (
        "resource",
        "`resource` selects which remote surface this mount reads, and changing it on a \
         live mount changes the id space its nodes are keyed on — every node is \
         re-imported under a new external id and the old ones are pruned. It is set in \
         the mount editor, where that consequence is stated, not by a config patch",
    ),
    (
        "principal",
        "`principal` selects WHOSE mailbox/drive this mount reads. Same re-import \
         consequence as `resource`; set it in the mount editor",
    ),
    (
        "batch_size",
        "`batch_size` and `batch_max_bytes` are a pair — one batch is one replication \
         record, and the byte budget is what keeps it under the transport frame cap. \
         Raise them together in the mount editor or not at all",
    ),
    (
        "batch_max_bytes",
        "`batch_max_bytes` and `batch_size` are a pair — see `batch_size`",
    ),
];

/// Validate and normalize one patch entry.
///
/// Normalizing matters: a JSON number arrives as `f64` often enough (a UI that
/// sends `300.0`) that storing it verbatim would put a float where the engine's
/// serde expects `u64`, and the mount would fail to parse — a config edit that
/// takes the whole mount down on the next run rather than at the request.
pub(crate) fn validate(field: &Field, value: &Value) -> Result<Value, String> {
    match field.kind {
        Kind::Bool => value
            .as_bool()
            .map(Value::from)
            .ok_or_else(|| format!("`{}` must be true or false", field.name)),
        Kind::Count { min, max } => count(field.name, value, min, max),
        Kind::NullableCount { min, max } => {
            if value.is_null() {
                // Explicit null CLEARS the key. The caller removes it rather
                // than storing JSON null, because the engine's serde reads an
                // absent key and a null one differently for `Option<u64>` only
                // by accident — an absent key is the state the defaults were
                // written for.
                return Ok(Value::Null);
            }
            count(field.name, value, min, max)
        }
        Kind::Text => value
            .as_str()
            .map(|s| Value::from(s.to_string()))
            .ok_or_else(|| format!("`{}` must be a string", field.name)),
        Kind::TextList => {
            let arr = value
                .as_array()
                .ok_or_else(|| format!("`{}` must be an array of strings", field.name))?;
            let mut out = Vec::with_capacity(arr.len());
            for item in arr {
                let s = item
                    .as_str()
                    .ok_or_else(|| format!("`{}` must be an array of strings", field.name))?;
                let s = s.trim();
                if !s.is_empty() {
                    out.push(Value::from(s.to_string()));
                }
            }
            Ok(Value::Array(out))
        }
        Kind::Mode => {
            let s = value
                .as_str()
                .ok_or_else(|| "`mode` must be one of \"poll\", \"webhook\", \"hybrid\"")?;
            match s {
                "poll" | "webhook" | "hybrid" => Ok(Value::from(s.to_string())),
                other => Err(format!(
                    "`mode` must be one of \"poll\", \"webhook\", \"hybrid\"; got \"{other}\". \
                     An unrecognized mode is treated as a poll mount by the scheduler, so it \
                     is refused here rather than becoming a setting that reads as chosen and \
                     behaves as the default"
                )),
            }
        }
    }
}

fn count(name: &str, value: &Value, min: u64, max: u64) -> Result<Value, String> {
    let n = value
        .as_u64()
        // A float that is a whole number is an ordinary way for a browser to
        // send an integer; anything else is a real type error.
        .or_else(|| match value.as_f64() {
            Some(f) if f.is_finite() && f >= 0.0 && f.fract() == 0.0 => Some(f as u64),
            _ => None,
        })
        .ok_or_else(|| format!("`{name}` must be a non-negative whole number"))?;
    if n < min || n > max {
        return Err(format!("`{name}` must be between {min} and {max}; got {n}"));
    }
    Ok(Value::from(n))
}

/// Look one key up in the allow-list.
pub(crate) fn writable(name: &str) -> Option<&'static Field> {
    WRITABLE.iter().find(|f| f.name == name)
}

/// The refusal reason for a known-but-unwritable key.
pub(crate) fn refusal(name: &str) -> Option<&'static str> {
    REFUSED
        .iter()
        .find(|(k, _)| *k == name)
        .map(|(_, why)| *why)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_whole_float_is_accepted_and_stored_as_an_integer() {
        let f = writable("interval_seconds").unwrap();
        // A browser number input sends 300 as a JSON float often enough that
        // storing it verbatim would put a float where the engine's serde wants
        // u64 — the mount would then fail to parse on its NEXT run, far from
        // the request that caused it.
        assert_eq!(validate(f, &json!(300.0)).unwrap(), json!(300));
        assert!(validate(f, &json!(300.5)).is_err());
    }

    #[test]
    fn interval_bounds_are_enforced() {
        let f = writable("interval_seconds").unwrap();
        assert!(validate(f, &json!(5)).is_err());
        assert!(validate(f, &json!(60)).is_ok());
        assert!(validate(f, &json!(999_999)).is_err());
    }

    #[test]
    fn an_unrecognized_mode_is_refused_not_demoted() {
        let f = writable("mode").unwrap();
        assert!(validate(f, &json!("webhooks")).is_err());
        assert!(validate(f, &json!("hybrid")).is_ok());
    }

    #[test]
    fn only_the_ttls_accept_null() {
        assert!(validate(writable("content_ttl_seconds").unwrap(), &json!(null)).is_ok());
        assert!(validate(writable("ttl_seconds").unwrap(), &json!(null)).is_ok());
        assert!(validate(writable("cache_content").unwrap(), &json!(null)).is_err());
        assert!(validate(writable("mode").unwrap(), &json!(null)).is_err());
    }

    #[test]
    fn zero_is_a_content_ttl_but_not_a_node_ttl() {
        // 0 drops cached bytes as soon as processing is done — a real setting.
        assert!(validate(writable("content_ttl_seconds").unwrap(), &json!(0)).is_ok());
        // A node TTL of 0 would delete every synced node on the next run.
        assert!(validate(writable("ttl_seconds").unwrap(), &json!(0)).is_err());
    }

    #[test]
    fn text_lists_drop_blanks_and_reject_non_strings() {
        let f = writable("include_patterns").unwrap();
        assert_eq!(
            validate(f, &json!(["*.pdf", "  ", " *.docx "])).unwrap(),
            json!(["*.pdf", "*.docx"])
        );
        assert!(validate(f, &json!(["*.pdf", 3])).is_err());
    }

    /// The engine-owned keys must not be reachable, whatever they are called.
    #[test]
    fn engine_owned_keys_are_not_writable() {
        for key in [
            "state",
            "integration_ref",
            "account_ref",
            "remote_root",
            "enabled",
        ] {
            assert!(writable(key).is_none(), "{key} must not be writable");
        }
    }

    /// A refused key must carry a reason. "Unknown field" is the answer that
    /// makes an inert setting indistinguishable from a missing one.
    #[test]
    fn refused_keys_explain_themselves() {
        assert!(refusal("accepts_content").is_some_and(|r| r.contains("capability")));
        assert!(refusal("nonsense").is_none());
    }
}
