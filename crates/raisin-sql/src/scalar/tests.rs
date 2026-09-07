use super::*;
use chrono::{TimeZone, Utc};

fn t(s: &str) -> Literal {
    Literal::Text(s.to_string())
}
fn i(v: i32) -> Literal {
    Literal::Int(v)
}
fn d(v: f64) -> Literal {
    Literal::Double(v)
}
fn ts(s: &str) -> Literal {
    Literal::Timestamp(parse_timestamp(s).unwrap())
}
fn ev(name: &str, args: &[Literal]) -> Literal {
    evaluate(name, args)
        .unwrap_or_else(|| panic!("no kernel {}", name))
        .unwrap_or_else(|e| panic!("{} failed: {}", name, e))
}
fn err(name: &str, args: &[Literal]) -> String {
    evaluate(name, args).unwrap().unwrap_err()
}
fn text_of(l: Literal) -> String {
    match l {
        Literal::Text(s) => s,
        other => panic!("expected text, got {:?}", other),
    }
}
fn f64_of(l: Literal) -> f64 {
    match l {
        Literal::Double(f) => f,
        other => panic!("expected double, got {:?}", other),
    }
}

#[test]
fn every_alias_resolves_and_names_are_unique() {
    let mut seen = std::collections::HashSet::new();
    for k in all_kernels() {
        assert!(seen.insert(k.name), "duplicate kernel {}", k.name);
        for a in k.aliases {
            assert!(seen.insert(*a), "duplicate alias {}", a);
            assert!(std::ptr::eq(lookup(a).unwrap(), k));
        }
    }
    assert!(lookup("ceiling").is_some());
}

#[test]
fn strict_null_propagation_and_non_strict() {
    assert_eq!(ev("ABS", &[Literal::Null]), Literal::Null);
    assert_eq!(ev("SUBSTR", &[t("abc"), Literal::Null]), Literal::Null);
    assert_eq!(ev("CONCAT", &[t("a"), Literal::Null, t("b")]), t("ab"));
    assert_eq!(ev("GREATEST", &[Literal::Null, i(2), i(5)]), i(5));
    assert_eq!(ev("GREATEST", &[Literal::Null]), Literal::Null);
}

#[test]
fn math_kernels() {
    assert_eq!(ev("ABS", &[i(-3)]), i(3));
    assert_eq!(ev("ABS", &[d(-2.5)]), d(2.5));
    assert_eq!(ev("CEIL", &[d(2.1)]), d(3.0));
    assert_eq!(ev("CEILING", &[d(-2.1)]), d(-2.0));
    assert_eq!(ev("FLOOR", &[d(2.9)]), d(2.0));
    assert_eq!(ev("TRUNC", &[d(2.987), i(2)]), d(2.98));
    assert_eq!(ev("TRUNC", &[d(-2.9)]), d(-2.0));
    assert_eq!(ev("ROUND", &[d(2.345), i(2)]), d(2.35));
    assert_eq!(ev("SIGN", &[i(-9)]), i(-1));
    assert_eq!(ev("SIGN", &[d(0.0)]), d(0.0));
    assert_eq!(ev("SQRT", &[i(16)]), d(4.0));
    assert!(err("SQRT", &[i(-1)]).contains("negative"));
    assert_eq!(ev("CBRT", &[i(27)]), d(3.0));
    assert_eq!(ev("POWER", &[i(2), i(10)]), d(1024.0));
    assert_eq!(ev("POW", &[i(2), i(3)]), d(8.0));
    assert!((f64_of(ev("EXP", &[i(1)])) - std::f64::consts::E).abs() < 1e-12);
    assert!((f64_of(ev("LN", &[d(std::f64::consts::E)])) - 1.0).abs() < 1e-12);
    assert_eq!(ev("LOG", &[i(1000)]), d(3.0));
    assert!((f64_of(ev("LOG", &[i(2), i(8)])) - 3.0).abs() < 1e-12);
    assert_eq!(ev("LOG10", &[i(100)]), d(2.0));
    assert!(err("LN", &[i(0)]).contains("non-positive"));
    assert_eq!(ev("MOD", &[i(7), i(3)]), Literal::BigInt(1));
    assert_eq!(ev("MOD", &[d(7.5), i(2)]), d(1.5));
    assert!(err("MOD", &[i(1), i(0)]).contains("zero"));
    assert_eq!(ev("DIV", &[i(7), i(2)]), Literal::BigInt(3));
    assert_eq!(ev("DIV", &[i(-7), i(2)]), Literal::BigInt(-3));
    assert_eq!(ev("PI", &[]), d(std::f64::consts::PI));
    let r = f64_of(ev("RANDOM", &[]));
    assert!((0.0..1.0).contains(&r));
    assert_ne!(ev("RANDOM", &[]), ev("RANDOM", &[]));
    assert_eq!(ev("GREATEST", &[i(1), d(2.5), i(2)]), d(2.5));
    assert_eq!(ev("LEAST", &[t("b"), t("a"), t("c")]), t("a"));
    assert!(err("LEAST", &[i(1), t("a")]).contains("cannot compare"));
    assert!(err("ABS", &[t("x")]).contains("must be numeric"));
    assert!(err("ABS", &[i(1), i(2)]).contains("expects 1 argument"));
}

#[test]
fn string_kernels() {
    assert_eq!(ev("LOWER", &[t("ÄB")]), t("äb"));
    assert_eq!(ev("UPPER", &[Literal::Path("/a".into())]), t("/A"));
    assert_eq!(ev("LENGTH", &[t("héllo")]), i(5));
    assert_eq!(ev("CHAR_LENGTH", &[t("")]), i(0));
    assert_eq!(
        ev("CONCAT", &[t("a"), i(1), Literal::Boolean(true)]),
        t("a1true")
    );
    assert_eq!(
        ev("CONCAT_WS", &[t("-"), t("a"), Literal::Null, t("b")]),
        t("a-b")
    );
    assert_eq!(ev("CONCAT_WS", &[Literal::Null, t("a")]), Literal::Null);
    assert_eq!(ev("SUBSTR", &[t("hello"), i(2), i(3)]), t("ell"));
    assert_eq!(ev("SUBSTR", &[t("hello"), i(0), i(3)]), t("he"));
    assert_eq!(ev("SUBSTRING", &[t("hello"), i(4)]), t("lo"));
    assert_eq!(ev("SUBSTR", &[t("héllo"), i(2), i(1)]), t("é"));
    assert_eq!(ev("SUBSTR", &[t("abc"), i(10)]), t(""));
    assert!(err("SUBSTR", &[t("abc"), i(1), i(-1)]).contains("negative"));
    assert_eq!(ev("REPLACE", &[t("a.b.c"), t("."), t("-")]), t("a-b-c"));
    assert_eq!(ev("STRPOS", &[t("héllo"), t("llo")]), i(3));
    assert_eq!(ev("STRPOS", &[t("abc"), t("x")]), i(0));
    assert_eq!(ev("LEFT", &[t("hello"), i(2)]), t("he"));
    assert_eq!(ev("LEFT", &[t("hello"), i(-2)]), t("hel"));
    assert_eq!(ev("RIGHT", &[t("hello"), i(2)]), t("lo"));
    assert_eq!(ev("RIGHT", &[t("hello"), i(-2)]), t("llo"));
    assert_eq!(ev("REPEAT", &[t("ab"), i(3)]), t("ababab"));
    assert_eq!(ev("REPEAT", &[t("ab"), i(-1)]), t(""));
    assert_eq!(ev("REVERSE", &[t("héllo")]), t("olléh"));
    assert_eq!(ev("SPLIT_PART", &[t("a,b,c"), t(","), i(2)]), t("b"));
    assert_eq!(ev("SPLIT_PART", &[t("a,b,c"), t(","), i(-1)]), t("c"));
    assert_eq!(ev("SPLIT_PART", &[t("a,b,c"), t(","), i(9)]), t(""));
    assert!(err("SPLIT_PART", &[t("a"), t(","), i(0)]).contains("zero"));
    assert_eq!(
        ev("STARTS_WITH", &[t("hello"), t("he")]),
        Literal::Boolean(true)
    );
    assert_eq!(
        ev("ENDS_WITH", &[t("hello"), t("x")]),
        Literal::Boolean(false)
    );
    assert_eq!(ev("INITCAP", &[t("hello wORLD-foo")]), t("Hello World-Foo"));
    assert_eq!(ev("ASCII", &[t("A")]), i(65));
    assert_eq!(ev("ASCII", &[t("")]), i(0));
    assert_eq!(ev("CHR", &[i(65)]), t("A"));
    assert!(err("CHR", &[i(0)]).contains("not a valid"));
    assert!(err("LOWER", &[i(1)]).contains("must be text"));
}

#[test]
fn trim_and_pad_kernels() {
    assert_eq!(ev("TRIM", &[t("  a b  ")]), t("a b"));
    assert_eq!(ev("BTRIM", &[t("xxaxx"), t("x")]), t("a"));
    assert_eq!(ev("LTRIM", &[t("  a  ")]), t("a  "));
    assert_eq!(ev("RTRIM", &[t("  a  ")]), t("  a"));
    assert_eq!(ev("LTRIM", &[t("xyxa"), t("xy")]), t("a"));
    assert_eq!(ev("LPAD", &[t("5"), i(3), t("0")]), t("005"));
    assert_eq!(ev("LPAD", &[t("abcdef"), i(3)]), t("abc"));
    assert_eq!(ev("RPAD", &[t("a"), i(4), t("xy")]), t("axyx"));
    assert_eq!(ev("RPAD", &[t("a"), i(3)]), t("a  "));
    assert_eq!(ev("LPAD", &[t("a"), i(3), t("")]), t("a"));
}

#[test]
fn hash_kernels() {
    assert_eq!(
        ev("MD5", &[t("hello")]),
        t("5d41402abc4b2a76b9719d911017c592")
    );
    assert_eq!(
        ev("SHA256", &[t("hello")]),
        t("2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824")
    );
    assert_eq!(ev("TO_HEX", &[i(255)]), t("ff"));
}

#[test]
fn regexp_kernels() {
    assert_eq!(
        ev("REGEXP_MATCH", &[t("foo123bar"), t("[0-9]+")]),
        Literal::JsonB(serde_json::json!(["123"]))
    );
    assert_eq!(
        ev("REGEXP_MATCH", &[t("2024-05"), t("(\\d+)-(\\d+)")]),
        Literal::JsonB(serde_json::json!(["2024", "05"]))
    );
    assert_eq!(ev("REGEXP_MATCH", &[t("abc"), t("\\d")]), Literal::Null);
    assert_eq!(
        ev("REGEXP_MATCH", &[t("ABC"), t("b"), t("i")]),
        Literal::JsonB(serde_json::json!(["B"]))
    );
    assert_eq!(
        ev("REGEXP_REPLACE", &[t("a1b2"), t("\\d"), t("#")]),
        t("a#b2")
    );
    assert_eq!(
        ev("REGEXP_REPLACE", &[t("a1b2"), t("\\d"), t("#"), t("g")]),
        t("a#b#")
    );
    assert_eq!(
        ev(
            "REGEXP_REPLACE",
            &[t("John Smith"), t("(\\w+) (\\w+)"), t("\\2, \\1")]
        ),
        t("Smith, John")
    );
    assert_eq!(ev("REGEXP_REPLACE", &[t("a"), t("a"), t("$1")]), t("$1"));
    assert_eq!(
        ev("REGEXP_LIKE", &[t("hello"), t("^h.*o$")]),
        Literal::Boolean(true)
    );
    assert_eq!(
        ev("REGEXP_LIKE", &[t("Hello"), t("^h")]),
        Literal::Boolean(false)
    );
    assert_eq!(
        ev("REGEXP_LIKE", &[t("Hello"), t("^h"), t("i")]),
        Literal::Boolean(true)
    );
    assert!(err("REGEXP_LIKE", &[t("a"), t("(")]).contains("invalid regular expression"));
    assert!(err("REGEXP_LIKE", &[t("a"), t("a"), t("z")]).contains("unknown regexp flag"));
}

#[test]
fn format_kernel() {
    assert_eq!(
        ev("FORMAT", &[t("Hello %s, you are %s"), t("Bob"), i(42)]),
        t("Hello Bob, you are 42")
    );
    assert_eq!(
        ev("FORMAT", &[t("%I = %L"), t("my col"), t("it's")]),
        t("\"my col\" = 'it''s'")
    );
    assert_eq!(ev("FORMAT", &[t("%I"), t("name")]), t("name"));
    assert_eq!(ev("FORMAT", &[t("%L"), Literal::Null]), t("NULL"));
    assert_eq!(ev("FORMAT", &[t("%s"), Literal::Null]), t(""));
    assert_eq!(ev("FORMAT", &[t("100%%")]), t("100%"));
    assert_eq!(ev("FORMAT", &[t("%2$s %1$s"), t("a"), t("b")]), t("b a"));
    assert_eq!(ev("FORMAT", &[Literal::Null, t("a")]), Literal::Null);
    assert!(err("FORMAT", &[t("%s %s"), t("a")]).contains("too few"));
    assert!(err("FORMAT", &[t("%I"), Literal::Null]).contains("NULL"));
}

#[test]
fn typeof_kernel() {
    assert_eq!(ev("PG_TYPEOF", &[i(1)]), t("integer"));
    assert_eq!(ev("TYPEOF", &[d(1.0)]), t("double precision"));
    assert_eq!(ev("TYPEOF", &[t("x")]), t("text"));
    assert_eq!(ev("TYPEOF", &[Literal::Null]), t("unknown"));
    assert_eq!(
        ev("TYPEOF", &[ts("2024-01-01")]),
        t("timestamp with time zone")
    );
}

#[test]
fn temporal_helpers() {
    assert_eq!(
        parse_timestamp("2024-03-05 14:07:09+02"),
        Some(Utc.with_ymd_and_hms(2024, 3, 5, 12, 7, 9).unwrap())
    );
    assert_eq!(
        parse_timestamp("2024-03-05"),
        Some(Utc.with_ymd_and_hms(2024, 3, 5, 0, 0, 0).unwrap())
    );
    assert_eq!(parse_timestamp("nope"), None);
    assert_eq!(
        format_interval(&chrono::Duration::seconds(93784)),
        "1 day 02:03:04"
    );
    assert_eq!(format_interval(&chrono::Duration::days(3)), "3 days");
    assert_eq!(format_interval(&chrono::Duration::zero()), "00:00:00");
    assert_eq!(
        format_interval(&chrono::Duration::milliseconds(-1500)),
        "-00:00:01.5"
    );
    assert_eq!(
        format_interval(&chrono::Duration::seconds(-90000)),
        "-1 day -01:00:00"
    );
}

#[test]
fn temporal_kernels() {
    let base = ts("2024-03-05T14:07:09.250Z");
    let at = |s: &str| ts(s);
    assert_eq!(
        ev("DATE_TRUNC", &[t("hour"), base.clone()]),
        at("2024-03-05T14:00:00Z")
    );
    assert_eq!(
        ev("DATE_TRUNC", &[t("day"), base.clone()]),
        at("2024-03-05")
    );
    assert_eq!(
        ev("DATE_TRUNC", &[t("week"), base.clone()]),
        at("2024-03-04")
    );
    assert_eq!(
        ev("DATE_TRUNC", &[t("month"), base.clone()]),
        at("2024-03-01")
    );
    assert_eq!(
        ev("DATE_TRUNC", &[t("quarter"), base.clone()]),
        at("2024-01-01")
    );
    assert_eq!(
        ev("DATE_TRUNC", &[t("year"), base.clone()]),
        at("2024-01-01")
    );
    assert_eq!(
        ev("DATE_TRUNC", &[t("decade"), base.clone()]),
        at("2020-01-01")
    );
    assert_eq!(
        ev("DATE_TRUNC", &[t("century"), base.clone()]),
        at("2001-01-01")
    );
    assert_eq!(
        ev("DATE_TRUNC", &[t("minute"), t("2024-03-05 14:07:09")]),
        at("2024-03-05T14:07:00Z")
    );
    assert!(err("DATE_TRUNC", &[t("fortnight"), base.clone()]).contains("unknown unit"));

    assert_eq!(ev("DATE_PART", &[t("year"), base.clone()]), d(2024.0));
    assert_eq!(ev("EXTRACT", &[t("MONTH"), base.clone()]), d(3.0));
    assert_eq!(ev("DATE_PART", &[t("day"), base.clone()]), d(5.0));
    assert_eq!(ev("DATE_PART", &[t("hour"), base.clone()]), d(14.0));
    assert_eq!(ev("DATE_PART", &[t("second"), base.clone()]), d(9.25));
    assert_eq!(ev("DATE_PART", &[t("epoch"), at("1970-01-02")]), d(86400.0));
    assert_eq!(ev("DATE_PART", &[t("dow"), base.clone()]), d(2.0));
    assert_eq!(ev("DATE_PART", &[t("isodow"), at("2024-03-03")]), d(7.0));
    assert_eq!(ev("DATE_PART", &[t("doy"), base.clone()]), d(65.0));
    assert_eq!(ev("DATE_PART", &[t("quarter"), base.clone()]), d(1.0));
    assert_eq!(ev("DATE_PART", &[t("week"), base.clone()]), d(10.0));
    assert_eq!(ev("DATE_PART", &[t("century"), base.clone()]), d(21.0));
    assert_eq!(
        ev(
            "DATE_PART",
            &[t("epoch"), Literal::Interval(chrono::Duration::minutes(90))]
        ),
        d(5400.0)
    );
    assert_eq!(
        ev(
            "DATE_PART",
            &[t("hour"), Literal::Interval(chrono::Duration::minutes(90))]
        ),
        d(1.0)
    );
    assert!(err("DATE_PART", &[t("fortnight"), base.clone()]).contains("unknown field"));

    assert_eq!(
        ev("AGE", &[at("2024-03-05"), at("2024-03-03T22:00:00Z")]),
        Literal::Interval(chrono::Duration::hours(26))
    );
    assert!(
        matches!(ev("AGE", &[at("2000-01-01")]), Literal::Interval(x) if x > chrono::Duration::zero())
    );

    assert_eq!(ev("TO_TIMESTAMP", &[i(86400)]), at("1970-01-02"));
    assert_eq!(ev("TO_TIMESTAMP", &[d(1.5)]), at("1970-01-01T00:00:01.5Z"));
    assert_eq!(
        ev(
            "TO_TIMESTAMP",
            &[t("05/03/2024 14:07"), t("DD/MM/YYYY HH24:MI")]
        ),
        at("2024-03-05T14:07:00Z")
    );
    assert_eq!(
        ev(
            "TO_TIMESTAMP",
            &[t("March 5, 2024 2:07 PM"), t("Month DD, YYYY HH:MI PM")]
        ),
        at("2024-03-05T14:07:00Z")
    );
    assert_eq!(
        ev("TO_DATE", &[t("2024-03-05 23:59"), t("YYYY-MM-DD HH24:MI")]),
        at("2024-03-05")
    );
    assert_eq!(
        ev("TO_DATE", &[t("05 Mar 2024"), t("DD Mon YYYY")]),
        at("2024-03-05")
    );
    assert!(err("TO_DATE", &[t("2024-13-01"), t("YYYY-MM-DD")]).contains("not a valid date"));
    assert_eq!(ev("MAKE_DATE", &[i(2024), i(3), i(5)]), at("2024-03-05"));
    assert_eq!(
        ev(
            "MAKE_TIMESTAMP",
            &[i(2024), i(3), i(5), i(14), i(7), d(9.25)]
        ),
        base.clone()
    );
    assert!(err("MAKE_DATE", &[i(2024), i(2), i(30)]).contains("not a valid date"));

    let f = |fmt: &str| text_of(ev("TO_CHAR", &[base.clone(), t(fmt)]));
    assert_eq!(f("YYYY-MM-DD HH24:MI:SS"), "2024-03-05 14:07:09");
    assert_eq!(f("Mon DD, YYYY"), "Mar 05, 2024");
    assert_eq!(f("FMMonth FMDD, YYYY"), "March 5, 2024");
    assert_eq!(f("Month"), "March    ");
    assert_eq!(f("MONTH"), "MARCH    ");
    assert_eq!(f("Day"), "Tuesday  ");
    assert_eq!(f("DY dy"), "TUE tue");
    assert_eq!(f("HH12:MI AM"), "02:07 PM");
    assert_eq!(f("HH:MI am"), "02:07 pm");
    assert_eq!(f("YY Q WW IW DDD D"), "24 1 10 10 065 3");
    assert_eq!(f("MS US"), "250 250000");
    assert_eq!(f("\"Year:\" YYYY TZ OF"), "Year: 2024 UTC +00");
    assert_eq!(f("J"), "2460375");
    assert_eq!(
        text_of(ev("TO_CHAR", &[t("2024-03-05T00:30:00Z"), t("HH12 AM")])),
        "12 AM"
    );

    assert!(matches!(
        ev("CURRENT_TIMESTAMP", &[]),
        Literal::Timestamp(_)
    ));
    assert!(matches!(ev("LOCALTIMESTAMP", &[]), Literal::Timestamp(_)));
    let today = match ev("CURRENT_DATE", &[]) {
        Literal::Timestamp(x) => x,
        _ => panic!(),
    };
    assert_eq!(today.format("%H:%M:%S").to_string(), "00:00:00");
    let now_text = text_of(ev("CURRENT_TIME", &[]));
    assert!(
        now_text.ends_with("+00") && now_text.len() == 18,
        "{}",
        now_text
    );
}

#[test]
fn fold_only_deterministic_and_successful() {
    assert_eq!(fold("ABS", &[i(-1)]), Some(i(1)));
    assert_eq!(fold("RANDOM", &[]), None);
    assert_eq!(fold("CURRENT_DATE", &[]), None);
    assert_eq!(fold("LN", &[i(0)]), None);
    assert_eq!(fold("NOPE", &[]), None);
}
