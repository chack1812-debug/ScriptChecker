"""THIRD-PARTY-NOTICES.txt を作る。実行ファイルにリンクされる crate(Windows 向け normal 依存)の
ライセンス表記と、各 crate 同梱のライセンス全文を書き出す。

使い方(プロジェクト直下で): python -I scripts/gen_third_party_notices.py
"""
import json
import pathlib
import re
import subprocess

root = pathlib.Path(__file__).resolve().parent.parent
TARGET = "x86_64-pc-windows-msvc"


def run(*args):
    return subprocess.run(args, cwd=root, check=True, capture_output=True, text=True, encoding="utf-8").stdout


tree = run("cargo", "tree", "-e", "normal", "--target", TARGET, "--prefix", "none", "--offline")
used = set()
for line in tree.splitlines():
    m = re.match(r"(\S+) v(\S+)", line.strip())
    if m:
        used.add((m.group(1), m.group(2)))

meta = json.loads(run("cargo", "metadata", "--format-version", "1", "--offline"))
pkgs = {(p["name"], p["version"]): p for p in meta["packages"]}
self_name = "scriptchecker"

LIC_NAMES = re.compile(r"^(licen[sc]e|copying|notice|unlicense)", re.I)

out = [
    "THIRD-PARTY NOTICES",
    "",
    "ScriptChecker(GPL-3.0-or-later)は、次のオープンソースのクレートを実行ファイルに含みます。",
    "各クレートの著作権表示とライセンス全文を以下に示します。",
    "",
    "== 一覧 ==",
    "",
]
entries = []
for key in sorted(used):
    if key[0] == self_name:
        continue
    p = pkgs.get(key)
    if p is None:
        continue
    entries.append(p)
    out.append(f"{p['name']} {p['version']}  [{p['license'] or p.get('license_file') or '不明'}]")
out.append("")
for p in entries:
    d = pathlib.Path(p["manifest_path"]).parent
    files = sorted(f for f in d.iterdir() if f.is_file() and LIC_NAMES.match(f.name))
    out.append("=" * 72)
    out.append(f"{p['name']} {p['version']}  [{p['license'] or '不明'}]")
    if p.get("repository"):
        out.append(p["repository"])
    out.append("=" * 72)
    if not files:
        out.append("(このクレートはライセンスファイルを同梱していません。上記のライセンス(MIT / Apache-2.0 等)に従います。")
        out.append(" 全文は、同じライセンスを掲げる他のクレートの項を参照してください)")
    for f in files:
        out.append(f"--- {f.name} ---")
        out.append(f.read_text(encoding="utf-8", errors="replace").strip())
        out.append("")
    out.append("")
(root / "THIRD-PARTY-NOTICES.txt").write_text("\n".join(out), encoding="utf-8")
print(f"{len(entries)} crates")
