#!/usr/bin/env python3
"""Sweep package CONTENT against built-in nodetype declarations.

This is the class of bug that breaks `deploy --install`: a content node whose
property value has a different SHAPE than its nodetype declares. Live-data
sweeps cannot find these -- the offending nodes are exactly the ones that failed
to install, so they are absent from the database.

Mirrors value_matches() in
raisin-core/src/services/node_validation/property_checks.rs.
"""
import glob, os, re, sys, yaml

GN = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                  "crates", "raisin-core", "global_nodetypes")
PKG = next((a for a in sys.argv[1:] if not a.startswith("-")), None)
if not PKG:
    sys.exit("usage: sweep-builtin-property-shapes.py <package-dir> [--unpatched]")
PATCHED = "--unpatched" not in sys.argv

# ---- declared types from the built-in nodetype YAMLs -------------------------
decls = {}
for fn in glob.glob(f"{GN}/*.yaml"):
    txt = open(fn).read()
    m = re.search(r'^name:\s*(\S+)', txt, re.M)
    if not m:
        continue
    nt = m.group(1).strip('"\'')
    props, cur = {}, None
    for line in txt.splitlines():
        pm = re.match(r'^  - name:\s*(\S+)', line)
        if pm:
            cur = pm.group(1).strip('"\''); continue
        tm = re.match(r'^    type:\s*(\S+)', line)
        if tm and cur:
            props[cur] = tm.group(1).strip('"\''); cur = None
    if props:
        decls[nt] = props

# ---- classify a YAML value as untagged PropertyValue would -------------------
def variant(v):
    if v is None: return "Null"
    if isinstance(v, bool): return "Boolean"
    if isinstance(v, int): return "Integer"
    if isinstance(v, float): return "Float"
    if isinstance(v, str): return "String"
    if isinstance(v, list):
        # Vector(Vec<f32>) precedes Array in the untagged enum, so an empty list
        # -- or a list of pure numbers -- resolves to Vector FIRST.
        if all(isinstance(x, (int, float)) and not isinstance(x, bool) for x in v):
            return "Vector"
        return "Array"
    if isinstance(v, dict):
        if "raisin:ref" in v: return "Reference"
        if "element_type" in v: return "Element"
        if {"uuid", "created_at", "updated_at"} <= set(v): return "Resource"
        return "Object"
    return "?"

OK = {
 "String": {"String"}, "NodeType": {"String"},
 "Float": {"Float","Integer"}, "Number": {"Float","Integer"},
 "Integer": {"Integer"}, "Decimal": {"Decimal","String"},
 "Boolean": {"Boolean"},
 "Date": {"Date","String"}, "URL": {"Url","String"},
 "Reference": {"Reference"}, "Resource": {"Resource"},
 "Composite": {"Composite"}, "Element": {"Element"}, "Geometry": {"Geometry"},
 "Array": {"Array","Vector"} if PATCHED else {"Array"},
 "Object": ({"Object","Composite","Element","Reference"} if PATCHED
            else {"Object","Composite","Element"}),
}

findings = {}
scanned = 0
for fn in glob.glob(f"{PKG}/content/**/.node.yaml", recursive=True):
    try:
        doc = yaml.safe_load(open(fn)) or {}
    except Exception:
        continue
    nt = doc.get("node_type")
    props = doc.get("properties") or {}
    if not nt or nt not in decls or not isinstance(props, dict):
        continue
    scanned += 1
    for p, v in props.items():
        declared = decls[nt].get(p)
        if not declared or v is None:
            continue
        act = variant(v)
        allowed = OK.get(declared)
        if allowed and act not in allowed:
            key = (nt, p, declared, act)
            findings.setdefault(key, []).append(os.path.relpath(fn, PKG))

print(f"scanned {scanned} content node(s) against {len(decls)} built-in nodetypes"
      f"  [{'patched' if PATCHED else 'PRE-PATCH'} rules]")
if not findings:
    print("  no shape mismatches")
else:
    print(f"  {len(findings)} MISMATCH(ES):")
    for (nt, p, d, a), files in sorted(findings.items(), key=lambda x: -len(x[1])):
        print(f"\n  {nt}.{p}: declared {d}, actual {a}   ({len(files)} node(s))")
        for f in files[:4]:
            print(f"      {f}")
        if len(files) > 4:
            print(f"      ... and {len(files)-4} more")
