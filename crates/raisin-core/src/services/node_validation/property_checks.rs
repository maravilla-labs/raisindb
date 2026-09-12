//! Property validation checks.
//!
//! Validates required properties, strict mode constraints, and unique property
//! constraints against NodeType schemas.

use raisin_error::{Error, Result};
use raisin_indexer::IndexQuery;
use raisin_models::nodes::properties::schema::PropertyType;
use raisin_models::nodes::properties::value::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::{NodeRepository, Storage, StorageScope};
use std::collections::HashMap;

use crate::services::node_type_resolver::ResolvedNodeType;

use super::core::NodeValidator;

impl<S: Storage> NodeValidator<S> {
    /// Check that all required properties are present
    pub(super) fn check_required_properties(
        &self,
        node: &Node,
        resolved: &ResolvedNodeType,
    ) -> Result<()> {
        for schema in &resolved.resolved_properties {
            let is_required = schema.required.unwrap_or(false);
            if is_required {
                let prop_name = schema
                    .name
                    .as_ref()
                    .ok_or_else(|| Error::Validation("Property schema has no name".to_string()))?;

                if !node.properties.contains_key(prop_name) {
                    return Err(Error::Validation(format!(
                        "Missing required property '{}' for NodeType '{}'",
                        prop_name, node.node_type
                    )));
                }
            }
        }
        Ok(())
    }

    /// Check that no undefined properties exist (strict mode)
    pub(super) fn check_strict_mode(&self, node: &Node, resolved: &ResolvedNodeType) -> Result<()> {
        // Build set of allowed property names
        let allowed_properties: HashMap<&str, ()> = resolved
            .resolved_properties
            .iter()
            .filter_map(|schema| schema.name.as_deref().map(|n| (n, ())))
            .collect();

        // Check each node property is defined in schema. Reserved ($-prefixed)
        // keys are server-computed metadata (e.g. $mixins) and are exempt.
        for key in node.properties.keys() {
            if raisin_models::nodes::is_reserved_property_key(key) {
                continue;
            }
            if !allowed_properties.contains_key(key.as_str()) {
                return Err(Error::Validation(format!(
                    "Undefined property '{}' in strict mode for NodeType '{}'",
                    key, node.node_type
                )));
            }
        }

        Ok(())
    }

    /// Check unique property constraints
    pub(super) async fn check_unique_properties(
        &self,
        workspace: &str,
        node: &Node,
        resolved: &ResolvedNodeType,
    ) -> Result<()> {
        let node_repo = self.storage.nodes();

        // Lazily-loaded previously-stored revision of this node (only fetched
        // if a unique property actually needs comparing). On update, a
        // unique property whose value is unchanged from the stored revision
        // is skipped: two nodes that already share a unique value (an
        // existing authoring collision, pre-dating this write) must not
        // block every future save of a completely unrelated field on either
        // node forever. Genuinely new/changed values are still checked.
        let mut previous: Option<Option<Node>> = None;

        for schema in &resolved.resolved_properties {
            if schema.unique.unwrap_or(false) {
                let prop_name = match &schema.name {
                    Some(n) => n,
                    None => continue,
                };

                // Check if this property has a value in the node
                if let Some(property_value) = node.properties.get(prop_name) {
                    if previous.is_none() {
                        let scope = StorageScope::new(
                            &self.tenant_id,
                            &self.repo_id,
                            &self.branch,
                            workspace,
                        );
                        previous = Some(node_repo.get(scope, &node.id, None).await?);
                    }

                    if let Some(Some(prev_node)) = &previous {
                        if let Some(prev_value) = prev_node.properties.get(prop_name) {
                            if Self::property_values_equal(property_value, prev_value) {
                                continue;
                            }
                        }
                    }

                    // Query for conflicting nodes
                    if let Some(conflicting) = self
                        .find_conflicting_node(
                            workspace,
                            &node.id,
                            prop_name,
                            property_value,
                            node_repo,
                        )
                        .await?
                    {
                        return Err(Error::Validation(format!(
                            "Property '{}' must be unique, but node '{}' (id: '{}') has the same value",
                            prop_name, conflicting.name, conflicting.id
                        )));
                    }
                }
            }
        }

        Ok(())
    }

    /// Find a node with conflicting unique property value
    async fn find_conflicting_node(
        &self,
        workspace: &str,
        current_node_id: &str,
        prop_name: &str,
        prop_value: &PropertyValue,
        node_repo: &S::Nodes,
    ) -> Result<Option<Node>> {
        // Try using index first if available (O(1) lookup)
        if let Some(ref index_mgr) = self.index_manager {
            // Use repository-scoped workspace key expected by PropertyIndexPlugin
            // Format: "{tenant}/{repo}/{branch}" (branchless storage, branch at service level)
            let workspace_key = format!("{}/{}/{}", self.tenant_id, self.repo_id, self.branch);
            let query = IndexQuery::FindByProperty {
                workspace: workspace_key,
                property_name: prop_name.to_string(),
                property_value: Box::new(prop_value.clone()),
            };

            // Query the property_unique index
            if let Ok(node_ids) = index_mgr.query("property_unique", query).await {
                // Check if any of the found nodes is different from current node
                for node_id in node_ids {
                    if node_id != current_node_id {
                        // Load the node to return it
                        let scope = StorageScope::new(
                            &self.tenant_id,
                            &self.repo_id,
                            &self.branch,
                            workspace,
                        );
                        if let Some(node) = node_repo.get(scope, &node_id, None).await? {
                            return Ok(Some(node));
                        }
                    }
                }
                return Ok(None);
            }
        }

        // Fallback to O(n) scan if index not available or query failed
        let scope = StorageScope::new(&self.tenant_id, &self.repo_id, &self.branch, workspace);
        let all_nodes = node_repo
            .list_all(scope, raisin_storage::ListOptions::for_api())
            .await?;

        for node in all_nodes {
            // Skip the current node
            if node.id == current_node_id {
                continue;
            }

            // Check if this node has the same property value
            if let Some(other_value) = node.properties.get(prop_name) {
                if Self::property_values_equal(prop_value, other_value) {
                    return Ok(Some(node));
                }
            }
        }

        Ok(None)
    }

    /// Compare two property values for equality
    pub(super) fn property_values_equal(a: &PropertyValue, b: &PropertyValue) -> bool {
        match (a, b) {
            (PropertyValue::String(s1), PropertyValue::String(s2)) => s1 == s2,
            (PropertyValue::Integer(n1), PropertyValue::Integer(n2)) => n1 == n2,
            (PropertyValue::Float(n1), PropertyValue::Float(n2)) => n1 == n2,
            (PropertyValue::Boolean(b1), PropertyValue::Boolean(b2)) => b1 == b2,
            (PropertyValue::Date(d1), PropertyValue::Date(d2)) => d1 == d2,
            (PropertyValue::Url(u1), PropertyValue::Url(u2)) => u1.url == u2.url,
            (PropertyValue::Reference(r1), PropertyValue::Reference(r2)) => r1.id == r2.id,
            (PropertyValue::Array(a1), PropertyValue::Array(a2)) => a1 == a2,
            (PropertyValue::Object(o1), PropertyValue::Object(o2)) => o1 == o2,
            (PropertyValue::Element(b1), PropertyValue::Element(b2)) => b1.uuid == b2.uuid,
            (PropertyValue::Composite(bc1), PropertyValue::Composite(bc2)) => bc1.uuid == bc2.uuid,
            (PropertyValue::Resource(r1), PropertyValue::Resource(r2)) => r1.uuid == r2.uuid,
            _ => false,
        }
    }
}

/// The name a `PropertyValue` variant reports in an error message.
fn value_kind(v: &PropertyValue) -> &'static str {
    match v {
        PropertyValue::Null => "null",
        PropertyValue::String(_) => "String",
        PropertyValue::Integer(_) => "Integer",
        PropertyValue::Float(_) => "Float",
        PropertyValue::Decimal(_) => "Decimal",
        PropertyValue::Boolean(_) => "Boolean",
        PropertyValue::Date(_) => "Date",
        PropertyValue::Url(_) => "URL",
        PropertyValue::Reference(_) => "Reference",
        PropertyValue::Resource(_) => "Resource",
        PropertyValue::Composite(_) => "Composite",
        PropertyValue::Element(_) => "Element",
        PropertyValue::Geometry(_) => "Geometry",
        PropertyValue::Array(_) => "Array",
        PropertyValue::Object(_) => "Object",
        PropertyValue::Vector(_) => "Vector",
    }
}

/// Does `value` satisfy a property declared as `declared`?
///
/// `Null` always passes — absence is how a property is cleared, and a declared
/// type says what a value must look like WHEN THERE IS ONE. `required` is a
/// separate check with its own error.
fn value_matches(declared: &PropertyType, value: &PropertyValue) -> bool {
    use PropertyType as T;
    match (declared, value) {
        (_, PropertyValue::Null) => true,

        (T::String, PropertyValue::String(_)) => true,
        // A type NAME is spelled as a string.
        (T::NodeType, PropertyValue::String(_)) => true,

        // An integer is a valid float; the reverse is not true.
        (T::Float, PropertyValue::Float(_) | PropertyValue::Integer(_)) => true,
        (T::Integer, PropertyValue::Integer(_)) => true,

        // EXACT decimal only. A Float here would mean the value already went
        // through an f64 — which is precisely what this type exists to prevent —
        // and an unparsed String means coercion did not run.
        (T::Decimal, PropertyValue::Decimal(_)) => true,

        (T::Boolean, PropertyValue::Boolean(_)) => true,

        // A Date accepts a STRING, and that is deliberate rather than lax.
        //
        // Over JSON a date IS a string, and `PropertyValue`'s untagged
        // deserialization does not turn a plain RFC3339 string into `Date` —
        // measured: `"2026-09-09T10:00:00Z"` arrives as `String`. Enforcing
        // `Date` strictly therefore refuses the ordinary spelling and would
        // break essentially every date write in every package.
        //
        // The real fix is to COERCE string -> Date the way declared decimals are
        // coerced above. It is deliberately not done here: `hash_property_value`
        // renders a Date as zero-padded nanoseconds rather than the raw string,
        // so coercing would change the property-index entries for every existing
        // date property and silently alter what equality and range queries
        // match. That is a coordinated coercion-plus-reindex, and it wants its
        // own change and its own migration.
        (T::Date, PropertyValue::Date(_) | PropertyValue::String(_)) => true,
        // A URL is routinely carried as a plain string, and both spellings
        // round-trip to the same thing.
        (T::URL, PropertyValue::Url(_) | PropertyValue::String(_)) => true,
        (T::Reference, PropertyValue::Reference(_)) => true,
        (T::Resource, PropertyValue::Resource(_)) => true,
        (T::Composite, PropertyValue::Composite(_)) => true,
        (T::Element, PropertyValue::Element(_)) => true,
        (T::Geometry, PropertyValue::Geometry(_)) => true,
        // An EMPTY ARRAY IS INDISTINGUISHABLE FROM AN EMPTY VECTOR, and
        // `PropertyValue` is untagged with `Vector(Vec<f32>)` declared BEFORE
        // `Array`, so serde resolves `[]` to `Vector([])` every time. Measured:
        // `raisin:Role.permissions: []` — the shipped `studio_member` default
        // role, which is deliberately empty — was refused on install with
        // "declared Array but the value is Vector". That is every empty array in
        // the system, not one role: an `Array` property is unwritable the moment
        // it holds nothing.
        //
        // Accepting Vector here is the narrow fix. Reordering the enum would
        // change how every existing array-of-numbers deserializes, and a real
        // embedding satisfies "is an array" anyway.
        (T::Array, PropertyValue::Array(_) | PropertyValue::Vector(_)) => true,
        // A Composite, an Element and a Reference are all object-shaped; a schema
        // that says `Object` is describing a bag, and all of them satisfy that
        // reading.
        //
        // Reference is included because a reference envelope
        // (`{raisin:ref, raisin:workspace}`) IS an object on the wire — only
        // `PropertyValue`'s untagged deserialization classifies it as Reference.
        // Measured: `raisin:Trigger.function_flow` is declared Object and its
        // consumer (`webhooks/execution.rs`) simply does `serde_json::to_value`
        // on whatever is there, so an inline flow object and a pointer to a
        // `raisin:Flow` node are both valid and both work. Enforcing Object
        // strictly refused the pointer spelling and rejected two shipped triggers
        // on install.
        (
            T::Object,
            PropertyValue::Object(_)
            | PropertyValue::Composite(_)
            | PropertyValue::Element(_)
            | PropertyValue::Reference(_),
        ) => true,

        _ => false,
    }
}

/// Parse the STRING spelling of every property declared `Decimal` into
/// `PropertyValue::Decimal`, and refuse a JSON number outright.
///
/// A decimal arrives as a string (`"19.90"`) because a JSON number has already
/// been through an f64 by the time any of our code sees it. Accepting one would
/// store a value that is quietly not what the caller sent, which defeats the
/// only reason to have the type. So the refusal is deliberate and names the
/// property, rather than coercing and hoping.
///
/// Runs BEFORE the type check below, so a well-formed string is already a
/// Decimal by the time anything inspects it.
pub(super) fn coerce_declared_decimals(node: &mut Node, resolved: &ResolvedNodeType) -> Result<()> {
    for prop in &resolved.resolved_properties {
        if prop.property_type != PropertyType::Decimal {
            continue;
        }
        let Some(name) = prop.name.as_deref() else {
            continue;
        };
        let Some(current) = node.properties.get(name) else {
            continue;
        };

        match current {
            PropertyValue::String(raw) => {
                let trimmed = raw.trim();
                if trimmed.is_empty() {
                    continue;
                }
                match trimmed.parse::<rust_decimal::Decimal>() {
                    Ok(d) => {
                        node.properties
                            .insert(name.to_string(), PropertyValue::Decimal(d));
                    }
                    Err(_) => {
                        return Err(Error::Validation(format!(
                            "Property '{}' is declared Decimal but '{}' is not a valid decimal \
                             number",
                            name, raw
                        )));
                    }
                }
            }
            PropertyValue::Float(_) | PropertyValue::Integer(_) => {
                return Err(Error::Validation(format!(
                    "Property '{}' is declared Decimal and must be sent as a STRING (e.g. \
                     \"19.90\"). A JSON number has already lost precision to a float before it \
                     reaches storage, so it is refused rather than silently rounded.",
                    name
                )));
            }
            // Already exact, or null/absent.
            _ => {}
        }
    }
    Ok(())
}

/// Parse an RFC3339 STRING on any property declared `Date` into
/// `PropertyValue::Date`.
///
/// This is not a convenience — without it, a `Date` property is unwritable from
/// anything that stores a string, and a `String` property holding a timestamp is
/// unwritable from anything that speaks JSON. `PropertyValue` is
/// `#[serde(untagged)]` with `Date` at slot 4, AHEAD of `String` at slot 6, so an
/// RFC3339 string arriving over the wire always deserializes as `Date` and can
/// never land as `String`. Meanwhile server-side code constructing
/// `PropertyValue::String(Utc::now().to_rfc3339())` in memory never goes through
/// that deserializer and stays a `String`. The same property therefore has two
/// different runtime types depending on which door the write came through.
///
/// While `type:` was documentation nothing noticed. Once `check_property_types`
/// enforced it, the two doors started refusing each other's values: the admin
/// console read an integration node, PUT its properties back unchanged, and got
///
///     Property 'capabilities_checked_at' on NodeType 'raisin:Integration'
///     is declared String but the value is Date
///
/// for a value the server itself had just written. The declarations are now
/// `Date` (which is what these values are, and what every other `_at` property
/// in the built-ins already says), and this pass reconciles the string spelling
/// so both doors agree.
///
/// It also covers data at rest: MessagePack stores a `Date` as a `[nanos]` tuple
/// and a `String` as a str, so values written before the declarations were
/// corrected really are strings in the blob. They coerce on their next write
/// instead of needing a migration.
///
/// Runs BEFORE the type check, for the same reason
/// [`coerce_declared_decimals`] does. A string that is NOT a valid timestamp is
/// refused by name rather than coerced and hoped over — same posture as the
/// decimal case.
pub(super) fn coerce_declared_dates(node: &mut Node, resolved: &ResolvedNodeType) -> Result<()> {
    for prop in &resolved.resolved_properties {
        if prop.property_type != PropertyType::Date {
            continue;
        }
        let Some(name) = prop.name.as_deref() else {
            continue;
        };
        let Some(PropertyValue::String(raw)) = node.properties.get(name) else {
            // Already a Date, or null/absent/some other type the type check
            // below will report.
            continue;
        };

        let trimmed = raw.trim();
        // An empty string is "unset", not a malformed timestamp. Refusing it
        // would make clearing an optional date impossible.
        if trimmed.is_empty() {
            continue;
        }

        match chrono::DateTime::parse_from_rfc3339(trimmed) {
            Ok(dt) => {
                let ts: raisin_models::timestamp::StorageTimestamp =
                    dt.with_timezone(&chrono::Utc).into();
                node.properties
                    .insert(name.to_string(), PropertyValue::Date(ts));
            }
            Err(_) => {
                return Err(Error::Validation(format!(
                    "Property '{}' is declared Date but '{}' is not a valid RFC3339 timestamp",
                    name, raw
                )));
            }
        }
    }

    coerce_dates_declared_as_string(node, resolved);
    Ok(())
}

/// The mirror image: a `Date` VALUE against a declaration that still says
/// `String`.
///
/// The pass above assumed the declarations would be corrected to `Date`
/// everywhere, and they were — in the binary. A NodeType lives in storage per
/// tenant and repo, though, and it only picks up a corrected declaration when it
/// resyncs. Between a deploy and that resync — or indefinitely, if a stale
/// definitions overlay outranks the binary — the server writes `Date` into a
/// schema that still declares `String`, and the type check refuses the write.
///
/// That is not a hypothetical. It took out one tenant's connector node
/// completely, and the damage was nowhere near the property involved:
///
/// - **Completed OAuth grants were discarded.** The user consented, Microsoft
///   issued tokens, and `oauth_callback` could not persist the node — so the
///   credential was thrown away after the irreversible half of the flow.
/// - **Every token refresh failed to save**, once a minute for hours, AFTER
///   performing a real token exchange at the provider. A provider that rotates
///   refresh tokens invalidates the stored one on each of those, so a write
///   that cannot land does not merely fail to save: it burns the credential.
///
/// Neither writer had touched `capabilities_checked_at`. They were refused
/// because of a value already sitting on the node — which is the real lesson
/// here, and why this coercion is worth having in both directions rather than
/// relying on every NodeType in every repo being current.
///
/// Rendering a timestamp as RFC3339 is loss-free and is exactly what the
/// `String` declaration meant, so there is nothing to refuse: unlike the
/// direction above, this cannot encounter an unparseable value.
fn coerce_dates_declared_as_string(node: &mut Node, resolved: &ResolvedNodeType) {
    for prop in &resolved.resolved_properties {
        if prop.property_type != PropertyType::String {
            continue;
        }
        let Some(name) = prop.name.as_deref() else {
            continue;
        };
        let Some(PropertyValue::Date(ts)) = node.properties.get(name) else {
            continue;
        };
        let rendered = ts.as_datetime().to_rfc3339();
        node.properties
            .insert(name.to_string(), PropertyValue::String(rendered));
    }
}

/// Enforce every declared `PropertyType` against the value actually present.
///
/// Historically `type:` on a NodeType property was documentation and an editor
/// hint — nothing compared a value against it — so this is the first check of
/// its kind and it can refuse content that previously saved. That is the point:
/// a `Decimal` that is really a float, or a `Date` that is really a string, is
/// exactly the drift that makes a bookkeeping total wrong later.
pub(super) fn check_property_types(node: &Node, resolved: &ResolvedNodeType) -> Result<()> {
    for prop in &resolved.resolved_properties {
        let Some(name) = prop.name.as_deref() else {
            continue;
        };
        let Some(value) = node.properties.get(name) else {
            continue;
        };
        if !value_matches(&prop.property_type, value) {
            return Err(Error::Validation(format!(
                "Property '{}' on NodeType '{}' is declared {:?} but the value is {}",
                name,
                node.node_type,
                prop.property_type,
                value_kind(value)
            )));
        }
    }
    Ok(())
}
