# Starlark arm of the cross-runtime benchmark. Mirrors bench.js and
# fixtures/wasm-guests/bench/src/lib.rs handler-for-handler.

def noop(input):
    return {}

def compute(input):
    # Modulo 2^32: Starlark integers are arbitrary precision, JS numbers are
    # f64 and the Rust guest uses u32, so the sum is normalised everywhere.
    sum = 0
    for i in range(input.get("n", 1)):
        sum = (sum + (i % 1000)) % 4294967296
    return {"sum": sum}

def hostcalls(input):
    ws = input.get("workspace", "content")
    path = input.get("path", "/pages")
    limit = input.get("limit", 3)
    n = input.get("n", 1)
    children = 0
    for i in range(n):
        children += len(raisin.nodes.get_children(ws, path, limit))
    return {"calls": n, "children": children}

def realistic(input):
    ws = input.get("workspace", "content")
    path = input.get("path", "/pages")
    limit = input.get("limit", 3)
    node_type = input.get("node_type", "raisin:Page")
    sql = input.get("sql", "SELECT id, name FROM 'content' WHERE node_type = $1")

    children = raisin.nodes.get_children(ws, path, limit)
    pages = 0
    for c in children:
        if c["node_type"] == node_type:
            pages += 1

    # `raisin.sql.query` returns a LIST of row dicts.
    rows = raisin.sql.query(sql, [node_type])

    return {"pages": pages, "rows": len(rows)}
