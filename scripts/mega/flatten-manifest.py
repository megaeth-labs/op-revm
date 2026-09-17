#!/usr/bin/env python3
"""Flatten the `workspace = true` inheritances of rust/op-revm/Cargo.toml.

Usage: flatten-manifest.py <monorepo-rust-dir> <crate Cargo.toml> > Cargo.toml

Reads the dependency requirements and package fields from `cargo metadata`
run in the monorepo workspace (which has the inheritance resolved), and the
`[workspace.lints.*]` tables from the workspace root manifest. Everything
else in the crate manifest, including comments and order, is kept as is, so
the diff against upstream stays reviewable.
"""
import json
import re
import subprocess
import sys

rust_dir, manifest = sys.argv[1], sys.argv[2]
meta = json.loads(
    subprocess.check_output(
        ["cargo", "metadata", "--format-version", "1", "--no-deps", "--offline"],
        cwd=rust_dir,
    )
)
pkg = next(p for p in meta["packages"] if p["name"] == "op-revm")
deps = {}
for d in pkg["dependencies"]:
    kind = d["kind"] or "normal"
    deps[(kind, d["name"])] = d


def toml_str(s):
    return '"' + s.replace('\\', '\\\\').replace('"', '\\"') + '"'


def render_dep(kind, name):
    d = deps[(kind, name)]
    req = d["req"].lstrip("^")
    feats = []
    for f in d["features"]:
        if f not in feats:
            feats.append(f)
    if d["uses_default_features"] and not feats and not d["optional"]:
        return f"{name} = {toml_str(req)}"
    parts = [f"version = {toml_str(req)}"]
    if not d["uses_default_features"]:
        parts.append("default-features = false")
    if feats:
        parts.append("features = [" + ", ".join(toml_str(f) for f in feats) + "]")
    if d["optional"]:
        parts.append("optional = true")
    return f"{name} = {{ {', '.join(parts)} }}"


def render_list(v):
    return "[" + ", ".join(toml_str(x) for x in v) + "]"


package_fields = {
    "authors": render_list(pkg["authors"]),
    "edition": toml_str(pkg["edition"]),
    "keywords": render_list(pkg["keywords"]),
    "license": toml_str(pkg["license"]),
    "repository": toml_str(pkg["repository"]),
    "rust-version": toml_str(pkg["rust_version"]),
    "homepage": toml_str(pkg.get("homepage") or ""),
    "categories": render_list(pkg.get("categories") or []),
}

root = open(f"{rust_dir}/Cargo.toml").read()
lints = []
capture = False
for line in root.splitlines():
    if re.match(r"^\[workspace\.lints\.", line):
        capture = True
        lints.append(line.replace("[workspace.lints.", "[lints."))
        continue
    if line.startswith("[") and capture:
        capture = False
    if capture:
        lints.append(line)
# the workspace root repeats the lints block; keep the first copy only
first = "\n".join(lints)
half = first.find("\n[lints.rust]", 1)
if half != -1:
    first = first[:half]
# drop trailing blank and comment lines that belong to the next section
kept = first.split("\n")
while kept and (not kept[-1].strip() or kept[-1].lstrip().startswith("#")):
    kept.pop()
lints_block = "\n".join(kept) + "\n"

out = []
section = None
for line in open(manifest).read().splitlines():
    m = re.match(r"^\[([^\]]+)\]", line)
    if m:
        section = m.group(1)
        if section == "lints":
            continue
        out.append(line)
        continue
    if section == "lints":
        if line.strip() == "workspace = true":
            out.append(lints_block.rstrip("\n"))
        elif line.strip():
            sys.exit(f"unexpected line in [lints]: {line}")
        else:
            out.append(line)
        continue
    m = re.match(r"^(\w[\w-]*)\.workspace = true\s*$", line)
    if m and section == "package":
        out.append(f"{m.group(1)} = {package_fields[m.group(1)]}")
        continue
    if m and section in ("dependencies", "dev-dependencies"):
        out.append(render_dep("normal" if section == "dependencies" else "dev", m.group(1)))
        continue
    m = re.match(r"^([\w-]+) = \{ workspace = true(?:, .*)? \}\s*$", line)
    if m and section in ("dependencies", "dev-dependencies"):
        out.append(render_dep("normal" if section == "dependencies" else "dev", m.group(1)))
        continue
    if "workspace = true" in line:
        sys.exit(f"unhandled inheritance: {line}")
    out.append(line)
print("\n".join(out))
