#!/usr/bin/env python3
"""差分测试：Python oracle vs Rust 实现。
用法: python scripts/diff_test.py <input.xlsx> [--redact]
语义比对策略 YAML（解析后比对，忽略注释/排版差异）。
"""
import subprocess, sys, tempfile, os, yaml, pathlib

PY = ["uv", "run", "finance-mask"]
RS = ["cargo", "run", "--quiet", "--manifest-path", "rust/Cargo.toml", "--"]

def gen(cmds, input, out):
    subprocess.run(cmds + ["generate", "-i", input, "-o", out], check=True,
                   capture_output=True)

VOLATILE = {"generated_at"}  # 时间戳每次运行必不同，比对前归一化

def semantic(path):
    d = yaml.safe_load(pathlib.Path(path).read_text(encoding="utf-8"))
    md = d.get("metadata") if isinstance(d, dict) else None
    if isinstance(md, dict):
        for k in VOLATILE:
            md[k] = "<TS>"   # 差分只比语义，时间戳归一化
    return d

def main():
    input = sys.argv[1]
    with tempfile.TemporaryDirectory() as d:
        py_out, rs_out = os.path.join(d, "py.yaml"), os.path.join(d, "rs.yaml")
        gen(PY, input, py_out)
        gen(RS, input, rs_out)
        py, rs = semantic(py_out), semantic(rs_out)
        if py == rs:
            print(f"DIFF-OK: {input} (sites={len(py.get('sites', []))})")
            return 0
        print(f"DIFF-FAIL: {input}")
        # 打印首个差异路径
        for k in set(py) | set(rs):
            if py.get(k) != rs.get(k):
                print(f"  field '{k}' differs")
        return 1

if __name__ == "__main__":
    sys.exit(main())
