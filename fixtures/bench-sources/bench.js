// JavaScript arm of the cross-runtime benchmark. Mirrors
// fixtures/wasm-guests/bench/src/lib.rs and bench.star handler-for-handler.

function noop(input) { return {}; }

function compute(input) {
    // Modulo 2^32 so all three numeric towers (f64 here, arbitrary-precision
    // in Starlark, u32 in Rust) agree exactly.
    let sum = 0;
    const n = input.n || 1;
    for (let i = 0; i < n; i++) { sum = (sum + (i % 1000)) >>> 0; }
    return { sum: sum };
}

function hostcalls(input) {
    const ws = input.workspace || "content";
    const path = input.path || "/pages";
    const limit = input.limit || 3;
    const n = input.n || 1;
    let children = 0;
    for (let i = 0; i < n; i++) {
        children += raisin.nodes.getChildren(ws, path, limit).length;
    }
    return { calls: n, children: children };
}

function realistic(input) {
    const ws = input.workspace || "content";
    const path = input.path || "/pages";
    const limit = input.limit || 3;
    const nodeType = input.node_type || "raisin:Page";
    const sql = input.sql || "SELECT id, name FROM 'content' WHERE node_type = $1";

    const children = raisin.nodes.getChildren(ws, path, limit);
    let pages = 0;
    for (const c of children) { if (c.node_type === nodeType) pages++; }

    // `raisin.sql.query` resolves to an ARRAY of row objects.
    const rows = raisin.sql.query(sql, [nodeType]);

    return { pages: pages, rows: rows.length };
}
