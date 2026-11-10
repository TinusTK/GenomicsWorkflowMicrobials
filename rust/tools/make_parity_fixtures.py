"""Writes rust/crates/gwm-core/tests/fixtures/<workflow>.json from the v1 Python
compiler, so the Rust compiler can be checked against it (cargo test).
Run from the repo root with an empty NTG_HOME:  NTG_HOME=$(mktemp -d) python3 rust/tools/make_parity_fixtures.py
"""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from app import catalog, compiler  # noqa: E402

out = Path(__file__).resolve().parents[1] / "crates/gwm-core/tests/fixtures"
out.mkdir(parents=True, exist_ok=True)
tools = catalog.load_tools()
for wf in catalog.builtin_workflows():
    try:
        steps = compiler.resolve(wf, tools)
    except compiler.CompileError as e:
        print(f"skip {wf['id']}: {e}")
        continue
    data = {
        "order": [r["id"] for r in steps],
        "settings": {r["id"]: {k: v if isinstance(v, str) else (str(v).lower() if isinstance(v, bool) else str(v))
                               for k, v in r["settings"].items()} for r in steps},
        "runnable": compiler.runnable(wf, tools),
    }
    (out / f"{wf['id']}.json").write_text(json.dumps(data, indent=2) + "\n")
    print(f"wrote {wf['id']}.json")
