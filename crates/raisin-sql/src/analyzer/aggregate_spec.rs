//! Aggregate call modifiers carried through the typed expression tree.
//!
//! `Expr::Function` has no slot for `DISTINCT` or an inner `ORDER BY`
//! (`COUNT(DISTINCT x)`, `ARRAY_AGG(x ORDER BY y DESC)`), and adding one would
//! touch every constructor in the crate. The analyzer therefore encodes the
//! modifiers into the function *name* and appends the ORDER BY expressions to
//! the argument list; everything that needs to recognise an aggregate goes
//! through [`AggregateSpec::parse`] / [`is_aggregate_name`] so the encoding
//! lives in exactly one place.
//!
//! Encoding: `BASE[ DISTINCT][ ORDER BY d1,d2,...]` where each `dN` is `a`
//! (ascending) or `d` (descending), one per trailing ORDER BY argument.

/// The six aggregate functions the planner understands.
pub const AGGREGATE_NAMES: [&str; 6] = ["COUNT", "SUM", "AVG", "MIN", "MAX", "ARRAY_AGG"];

/// Parsed form of an (possibly modifier-encoded) aggregate function name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AggregateSpec {
    /// Upper-case base name (`COUNT`, `ARRAY_AGG`, ...).
    pub base: String,
    /// `DISTINCT` was given.
    pub distinct: bool,
    /// One entry per inner ORDER BY key, `true` for DESC. The keys themselves
    /// are the last `order_desc.len()` arguments of the call.
    pub order_desc: Vec<bool>,
}

impl AggregateSpec {
    /// Parse a function name. Returns `None` for a non-aggregate.
    pub fn parse(name: &str) -> Option<Self> {
        let upper = name.to_uppercase();
        let (base, rest) = match upper.split_once(' ') {
            Some((b, r)) => (b.to_string(), r.trim().to_string()),
            None => (upper.clone(), String::new()),
        };
        if !AGGREGATE_NAMES.contains(&base.as_str()) {
            return None;
        }
        let mut distinct = false;
        let mut order_desc = Vec::new();
        let mut rest = rest.as_str();
        if let Some(r) = rest.strip_prefix("DISTINCT") {
            distinct = true;
            rest = r.trim();
        }
        if let Some(r) = rest.strip_prefix("ORDER BY") {
            order_desc = r
                .trim()
                .split(',')
                .filter(|s| !s.is_empty())
                .map(|s| s == "d")
                .collect();
        }
        Some(Self {
            base,
            distinct,
            order_desc,
        })
    }

    /// Encode back into a function name.
    pub fn encode(&self) -> String {
        let mut name = self.base.clone();
        if self.distinct {
            name.push_str(" DISTINCT");
        }
        if !self.order_desc.is_empty() {
            name.push_str(" ORDER BY ");
            let dirs: Vec<&str> = self
                .order_desc
                .iter()
                .map(|d| if *d { "d" } else { "a" })
                .collect();
            name.push_str(&dirs.join(","));
        }
        name
    }

    /// Number of trailing arguments that are ORDER BY keys, not values.
    pub fn order_arg_count(&self) -> usize {
        self.order_desc.len()
    }
}

/// True if `name` is one of the aggregate functions, with or without modifiers.
pub fn is_aggregate_name(name: &str) -> bool {
    AggregateSpec::parse(name).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_names_parse_without_modifiers() {
        let s = AggregateSpec::parse("count").unwrap();
        assert_eq!(s.base, "COUNT");
        assert!(!s.distinct);
        assert!(s.order_desc.is_empty());
        assert!(AggregateSpec::parse("UPPER").is_none());
    }

    #[test]
    fn encode_and_parse_round_trip() {
        let s = AggregateSpec {
            base: "ARRAY_AGG".into(),
            distinct: true,
            order_desc: vec![true, false],
        };
        let name = s.encode();
        assert_eq!(name, "ARRAY_AGG DISTINCT ORDER BY d,a");
        assert_eq!(AggregateSpec::parse(&name).unwrap(), s);
        assert!(is_aggregate_name(&name));
        assert_eq!(
            AggregateSpec::parse("COUNT DISTINCT").unwrap().distinct,
            true
        );
    }
}
