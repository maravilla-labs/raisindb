// SPDX-License-Identifier: BSL-1.1

//! Query building utilities for Tantivy search.

use raisin_hlc::HLC;
use tantivy::schema::Field;

pub(crate) fn wildcard_to_regex(pattern: &str) -> String {
    let mut regex = String::with_capacity(pattern.len() * 2);
    for ch in pattern.chars() {
        match ch {
            '*' => regex.push_str(".*"),
            '?' => regex.push('.'),
            '.' | '+' | '^' | '$' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '\\' => {
                regex.push('\\');
                regex.push(ch);
            }
            _ => regex.push(ch),
        }
    }
    regex
}

pub(crate) fn contains_wildcards(query: &str) -> bool {
    query.contains('*') || query.contains('?')
}

pub(crate) fn build_hlc_le_query(
    timestamp_field: Field,
    counter_field: Field,
    max_hlc: &HLC,
) -> Box<dyn tantivy::query::Query> {
    use std::ops::Bound;
    use tantivy::query::{BooleanQuery, Occur, RangeQuery};
    use tantivy::Term;

    let timestamp_less = RangeQuery::new(
        Bound::Unbounded,
        Bound::Excluded(Term::from_field_u64(timestamp_field, max_hlc.timestamp_ms)),
    );
    let timestamp_equal = RangeQuery::new(
        Bound::Included(Term::from_field_u64(timestamp_field, max_hlc.timestamp_ms)),
        Bound::Included(Term::from_field_u64(timestamp_field, max_hlc.timestamp_ms)),
    );
    let counter_le = RangeQuery::new(
        Bound::Unbounded,
        Bound::Included(Term::from_field_u64(counter_field, max_hlc.counter)),
    );

    let timestamp_eq_and_counter_le = BooleanQuery::new(vec![
        (
            Occur::Must,
            Box::new(timestamp_equal) as Box<dyn tantivy::query::Query>,
        ),
        (
            Occur::Must,
            Box::new(counter_le) as Box<dyn tantivy::query::Query>,
        ),
    ]);

    Box::new(BooleanQuery::new(vec![
        (
            Occur::Should,
            Box::new(timestamp_less) as Box<dyn tantivy::query::Query>,
        ),
        (
            Occur::Should,
            Box::new(timestamp_eq_and_counter_le) as Box<dyn tantivy::query::Query>,
        ),
    ]))
}

#[allow(dead_code)]
pub(crate) fn build_hlc_eq_query(
    timestamp_field: Field,
    counter_field: Field,
    target_hlc: &HLC,
) -> Box<dyn tantivy::query::Query> {
    use std::ops::Bound;
    use tantivy::query::{BooleanQuery, Occur, RangeQuery};
    use tantivy::Term;

    let timestamp_eq = RangeQuery::new(
        Bound::Included(Term::from_field_u64(
            timestamp_field,
            target_hlc.timestamp_ms,
        )),
        Bound::Included(Term::from_field_u64(
            timestamp_field,
            target_hlc.timestamp_ms,
        )),
    );
    let counter_eq = RangeQuery::new(
        Bound::Included(Term::from_field_u64(counter_field, target_hlc.counter)),
        Bound::Included(Term::from_field_u64(counter_field, target_hlc.counter)),
    );

    Box::new(BooleanQuery::new(vec![
        (
            Occur::Must,
            Box::new(timestamp_eq) as Box<dyn tantivy::query::Query>,
        ),
        (
            Occur::Must,
            Box::new(counter_eq) as Box<dyn tantivy::query::Query>,
        ),
    ]))
}
