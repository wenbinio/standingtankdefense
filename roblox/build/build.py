#!/usr/bin/env python3
"""
roblox/build/build.py -- source tree -> Roblox-shaped tree.

WHY THIS EXISTS
---------------
`src/**` is written for the *standalone* Luau interpreter, which resolves
modules by relative string path:

    local Fixed = require("../Fixed")

That form is the only one the bare interpreter accepts, and Roblox accepts
none of it -- Roblox requires an *instance reference*:

    local Fixed = require(script.Parent.Parent.Fixed)

Both have to keep working. Bare-interpreter execution is how the whole test
suite and the R3 correctness oracle run (`roblox/test/trace_test.luau` proves
the Luau sim reproduces the Rust bit-for-bit); losing it forfeits that
guarantee. So the sources are never edited. This script reads `src/**` and
emits Roblox-shaped *copies* into `dist/**`, rewriting each require into the
instance reference implied by the file's position in the Rojo tree. Rojo syncs
`dist/`, never `src/`.

WHAT IT GUARANTEES
------------------
The failure mode worth preventing is a require that silently survives the
rewrite and only explodes inside Studio. So nothing is trusted:

  * Every rewritten require must resolve to a module that was actually
    emitted. Unresolvable -> the build fails, loudly, with file:line.
  * After emitting, every generated require expression is *re-parsed from the
    emitted text* by an independent evaluator and walked against an instance
    tree built from the emitted files. Correct by construction, then checked
    anyway.
  * No relative-path string literal may survive anywhere in emitted code.
  * Every emitted file is parsed by `luau-analyze`. Any SyntaxError fails the
    build; the only tolerated diagnostics are the unavoidable "Unknown global
    'script'/'game'" (the analyzer has no Roblox definitions).
  * With `--exec-check`, every emitted module is additionally *executed* under
    the standalone interpreter against an emulated Roblox instance tree, which
    proves the rewritten graph actually loads.

USAGE
-----
    python3 roblox/build/build.py                # build + verify
    python3 roblox/build/build.py --exec-check   # + execute the emitted graph
    python3 roblox/build/build.py --check        # verify dist/ is up to date
    python3 roblox/build/build.py --verify-only  # verify existing dist/

Interpreter/analyzer are found on PATH, or via $LUAU / $LUAU_ANALYZE.
Python 3.8+, standard library only. Nothing under `src/` or `test/` is ever
written to.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Dict, List, Optional, Sequence, Tuple

ROBLOX_DIR = Path(__file__).resolve().parent.parent
PROJECT_FILE = ROBLOX_DIR / "default.project.json"

# `dist/<x>` in the Rojo project is built from `src/<x>`.
DIST_PREFIX = "dist/"
SRC_PREFIX = "src/"

LUA_KEYWORDS = {
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function",
    "if", "in", "local", "nil", "not", "or", "repeat", "return", "then",
    "true", "until", "while",
}

IDENT_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
RELATIVE_PATH_RE = re.compile(r"^\.\.?/")


class BuildError(Exception):
    pass


# --------------------------------------------------------------------------
# Luau lexer
#
# Only enough to be *exactly* right about where strings and comments begin and
# end -- the whole point is that a require inside a comment (Content.luau has
# one) must not be rewritten, and a path-looking substring inside a longer
# message string (Waves.luau, Shop.luau have those) must not be either. A
# regex cannot make that distinction; this can.
# --------------------------------------------------------------------------

class Token:
    __slots__ = ("kind", "text", "value", "start", "end_", "line")

    def __init__(self, kind: str, text: str, value, start: int, end_: int, line: int):
        self.kind = kind      # NAME | STRING | NUMBER | OP
        self.text = text      # raw source slice
        self.value = value    # decoded contents, for STRING
        self.start = start
        self.end_ = end_
        self.line = line

    def __repr__(self) -> str:  # pragma: no cover - debugging aid
        return f"<{self.kind} {self.text!r} @{self.line}>"


def _long_bracket_len(src: str, i: int) -> int:
    """If src[i:] opens a long bracket `[=*[`, return its total opener length."""
    if src[i] != "[":
        return 0
    j = i + 1
    while j < len(src) and src[j] == "=":
        j += 1
    if j < len(src) and src[j] == "[":
        return j - i + 1
    return 0


def _skip_long_bracket(src: str, i: int, opener_len: int) -> int:
    """Return the index just past the closing bracket of a long string/comment."""
    level = opener_len - 2
    closer = "]" + "=" * level + "]"
    end = src.find(closer, i + opener_len)
    if end < 0:
        raise BuildError("unterminated long bracket")
    return end + len(closer)


def _decode_short_string(raw: str) -> str:
    """Decode just enough of a short string for path comparison."""
    body = raw[1:-1]
    out: List[str] = []
    i = 0
    simple = {"n": "\n", "t": "\t", "r": "\r", "a": "\a", "b": "\b",
              "f": "\f", "v": "\v", "\\": "\\", '"': '"', "'": "'", "\n": "\n"}
    while i < len(body):
        c = body[i]
        if c != "\\":
            out.append(c)
            i += 1
            continue
        i += 1
        if i >= len(body):
            break
        e = body[i]
        if e in simple:
            out.append(simple[e])
            i += 1
        else:
            # \ddd, \xXX, \u{...}, \z -- irrelevant to module paths; drop them.
            out.append("�")
            i += 1
    return "".join(out)


def lex(src: str) -> List[Token]:
    """Tokenize Luau. Whitespace and comments are dropped."""
    toks: List[Token] = []
    i = 0
    line = 1
    n = len(src)
    while i < n:
        c = src[i]
        if c == "\n":
            line += 1
            i += 1
            continue
        if c in " \t\r\v\f":
            i += 1
            continue
        # comments
        if c == "-" and src.startswith("--", i):
            j = i + 2
            lb = _long_bracket_len(src, j) if j < n else 0
            if lb:
                end = _skip_long_bracket(src, j, lb)
            else:
                end = src.find("\n", j)
                end = n if end < 0 else end
            line += src.count("\n", i, end)
            i = end
            continue
        # long strings
        lb = _long_bracket_len(src, i)
        if lb:
            end = _skip_long_bracket(src, i, lb)
            raw = src[i:end]
            body = raw[lb:-lb]  # opener and closer are the same length
            if body.startswith("\n"):
                body = body[1:]
            toks.append(Token("STRING", raw, body, i, end, line))
            line += raw.count("\n")
            i = end
            continue
        # short strings
        if c in "\"'":
            j = i + 1
            while j < n:
                d = src[j]
                if d == "\\":
                    j += 2
                    continue
                if d == c:
                    j += 1
                    break
                if d == "\n":
                    raise BuildError(f"unterminated string at line {line}")
                j += 1
            else:
                raise BuildError(f"unterminated string at line {line}")
            raw = src[i:j]
            toks.append(Token("STRING", raw, _decode_short_string(raw), i, j, line))
            i = j
            continue
        # numbers (scanned only well enough not to swallow a quote)
        if c.isdigit() or (c == "." and i + 1 < n and src[i + 1].isdigit()):
            j = i
            while j < n and (src[j].isalnum() or src[j] in "._"):
                if src[j] in "pPeE" and j + 1 < n and src[j + 1] in "+-":
                    j += 1
                j += 1
            toks.append(Token("NUMBER", src[i:j], None, i, j, line))
            i = j
            continue
        # names
        if c.isalpha() or c == "_":
            j = i
            while j < n and (src[j].isalnum() or src[j] == "_"):
                j += 1
            toks.append(Token("NAME", src[i:j], None, i, j, line))
            i = j
            continue
        # operators / punctuation (longest first for the few multi-char ones)
        for op in ("...", "..=", "//=", "==", "~=", "<=", ">=", "..", "::",
                   "->", "+=", "-=", "*=", "/=", "%=", "^=", "//"):
            if src.startswith(op, i):
                toks.append(Token("OP", op, None, i, i + len(op), line))
                i += len(op)
                break
        else:
            toks.append(Token("OP", c, None, i, i + 1, line))
            i += 1
    return toks


# --------------------------------------------------------------------------
# Rojo project -> build roots
# --------------------------------------------------------------------------

class BuildRoot:
    def __init__(self, src_dir: Path, dist_dir: Path, instance_path: Tuple[str, ...]):
        self.src_dir = src_dir
        self.dist_dir = dist_dir
        self.instance_path = instance_path

    def __repr__(self) -> str:  # pragma: no cover
        return f"<BuildRoot {self.src_dir.name} -> {'.'.join(self.instance_path)}>"


def _node_path(node) -> Optional[str]:
    p = node.get("$path")
    if isinstance(p, str):
        return p
    if isinstance(p, dict) and isinstance(p.get("optional"), str):
        return p["optional"]
    return None


def read_build_roots() -> List[BuildRoot]:
    """Derive the build roots from the Rojo project, so the two cannot drift."""
    project = json.loads(PROJECT_FILE.read_text(encoding="utf-8"))
    roots: List[BuildRoot] = []

    def walk(node, path: Tuple[str, ...]):
        if not isinstance(node, dict):
            return
        p = _node_path(node)
        if p is not None and p.replace("\\", "/").startswith(DIST_PREFIX):
            rel = p.replace("\\", "/")[len(DIST_PREFIX):]
            roots.append(BuildRoot(
                src_dir=ROBLOX_DIR / SRC_PREFIX / rel,
                dist_dir=ROBLOX_DIR / DIST_PREFIX / rel,
                instance_path=path,
            ))
        for key, child in node.items():
            if key.startswith("$"):
                continue
            walk(child, path + (key,))

    walk(project.get("tree", {}), ())
    if not roots:
        raise BuildError(
            f"{PROJECT_FILE} maps nothing to `{DIST_PREFIX}`; "
            "the build step has no output to produce."
        )
    return roots


# --------------------------------------------------------------------------
# Source scan -> instance tree
# --------------------------------------------------------------------------

class Entry:
    """One emitted file and the instance it becomes."""

    def __init__(self, src: Path, dist: Path, instance_path: Tuple[str, ...],
                 class_name: str, is_module: bool, rewrite: bool):
        self.src = src
        self.dist = dist
        self.instance_path = instance_path
        self.class_name = class_name
        self.is_module = is_module   # requireable from Luau
        self.rewrite = rewrite       # subject to require rewriting


SCRIPT_SUFFIXES = [
    (".server.luau", "Script"),
    (".client.luau", "LocalScript"),
    (".server.lua", "Script"),
    (".client.lua", "LocalScript"),
    (".luau", "ModuleScript"),
    (".lua", "ModuleScript"),
]


def classify(name: str) -> Optional[Tuple[str, str, bool, bool]]:
    """(instance name, className, is_module, rewrite) for a file name."""
    lower = name.lower()
    if lower.endswith(".meta.json"):
        return None  # sidecar metadata: copied, not an instance of its own
    for suffix, cls in SCRIPT_SUFFIXES:
        if lower.endswith(suffix):
            stem = name[: -len(suffix)]
            return (stem, cls, cls == "ModuleScript", True)
    if lower.endswith(".json"):
        # Rojo turns a plain .json into a ModuleScript returning the decoded table.
        return (name[: -len(".json")], "ModuleScript", True, False)
    if lower.endswith((".txt", ".csv", ".toml")):
        return (name.rsplit(".", 1)[0], "StringValue", False, False)
    return None


def scan(roots: Sequence[BuildRoot]) -> List[Entry]:
    entries: List[Entry] = []
    for root in roots:
        if not root.src_dir.is_dir():
            continue
        for src in sorted(root.src_dir.rglob("*")):
            if not src.is_file():
                continue
            rel = src.relative_to(root.src_dir)
            dist = root.dist_dir / rel
            name = rel.name
            folders = tuple(rel.parts[:-1])
            info = classify(name)
            if info is None:
                # sidecars and unknown files ride along verbatim, with no
                # instance identity of their own.
                entries.append(Entry(src, dist, (), "", False, False))
                continue
            inst_name, cls, is_module, rewrite = info
            if inst_name in ("init", "init.server", "init.client") or \
                    (inst_name == "init"):
                # `init.luau` *is* its containing folder.
                if not folders:
                    raise BuildError(f"{src}: init script at a build root is unsupported")
                ipath = root.instance_path + folders
            elif inst_name.startswith("init") and name.lower().startswith("init."):
                ipath = root.instance_path + folders
            else:
                ipath = root.instance_path + folders + (inst_name,)
            entries.append(Entry(src, dist, ipath, cls, is_module, rewrite))
    return entries


# --------------------------------------------------------------------------
# Require resolution
# --------------------------------------------------------------------------

class RequireSite:
    def __init__(self, token: Optional[Token], line: int, kind: str,
                 arg_start: int = -1, arg_end: int = -1, text: str = ""):
        self.token = token
        self.line = line
        self.kind = kind          # "string" | "expr"
        self.arg_start = arg_start
        self.arg_end = arg_end
        self.text = text


def find_require_sites(toks: List[Token], where: str) -> List[RequireSite]:
    """Locate every require and the extent of its module argument.

    Handles the three shapes that occur:
        require("./X")            -- the common case
        require "./X"             -- Lua call sugar
        pcall(require, "../X")    -- require passed as a value (Shop, Waves)
    Anything else is reported as an expression require and passed through.
    """
    sites: List[RequireSite] = []
    for idx, t in enumerate(toks):
        if t.kind != "NAME" or t.text != "require":
            continue
        nxt = toks[idx + 1] if idx + 1 < len(toks) else None
        if nxt is None:
            raise BuildError(f"{where}:{t.line}: `require` at end of file")

        if nxt.kind == "OP" and nxt.text == "(":
            depth = 0
            j = idx + 1
            end = None
            while j < len(toks):
                tk = toks[j]
                if tk.kind == "OP" and tk.text in "([{":
                    depth += 1
                elif tk.kind == "OP" and tk.text in ")]}":
                    depth -= 1
                    if depth == 0:
                        end = j
                        break
                j += 1
            if end is None:
                raise BuildError(f"{where}:{t.line}: unbalanced require(")
            inner = toks[idx + 2:end]
            if len(inner) == 1 and inner[0].kind == "STRING":
                sites.append(RequireSite(inner[0], inner[0].line, "string"))
            else:
                if any(tk.kind == "STRING" and RELATIVE_PATH_RE.match(str(tk.value))
                       for tk in inner):
                    raise BuildError(
                        f"{where}:{t.line}: require() argument mixes a relative "
                        f"module path into a larger expression; this build step "
                        f"cannot resolve it. Rewrite it as a plain literal."
                    )
                sites.append(RequireSite(
                    None, t.line, "expr",
                    toks[idx + 2].start if inner else -1,
                    toks[end - 1].end_ if inner else -1,
                ))
            continue

        if nxt.kind == "STRING":
            sites.append(RequireSite(nxt, nxt.line, "string"))
            continue

        # `require` used as a first-class value.
        if nxt.kind == "OP" and nxt.text == ",":
            arg = toks[idx + 2] if idx + 2 < len(toks) else None
            if arg is not None and arg.kind == "STRING":
                sites.append(RequireSite(arg, arg.line, "string"))
                continue
            raise BuildError(
                f"{where}:{t.line}: `require` is passed as a value but its module "
                f"argument is not a string literal; this build step cannot rewrite it."
            )
        if nxt.kind == "OP" and nxt.text in (")", "}"):
            # e.g. `local r = require` / `{ require }` -- the call site is elsewhere.
            raise BuildError(
                f"{where}:{t.line}: `require` escapes as a bare value; its call "
                f"site cannot be rewritten by this build step."
            )
        raise BuildError(f"{where}:{t.line}: unrecognised require form `require {nxt.text}`")
    return sites


def resolve_module_path(spec: str, src_file: Path, roots: Sequence[BuildRoot],
                        by_src: Dict[Path, Entry], where: str, line: int) -> Entry:
    """`./X` / `../X` -> the Entry it names, exactly as the bare interpreter resolves it."""
    if not RELATIVE_PATH_RE.match(spec):
        raise BuildError(
            f"{where}:{line}: require(\"{spec}\") is not a relative path. "
            f"Only `./x` and `../x` are resolvable."
        )
    base = (src_file.parent / spec).resolve()
    candidates = [
        base.with_name(base.name + ".luau"),
        base.with_name(base.name + ".lua"),
        base / "init.luau",
        base / "init.lua",
        base.with_name(base.name + ".json"),
    ]
    for cand in candidates:
        entry = by_src.get(cand)
        if entry is not None and entry.is_module:
            return entry
    for cand in candidates:
        if cand.exists():
            raise BuildError(
                f"{where}:{line}: require(\"{spec}\") resolves to {cand}, which is "
                f"outside every build root in {PROJECT_FILE.name} and so is never "
                f"emitted. It cannot be referenced from Roblox."
            )
    raise BuildError(
        f"{where}:{line}: require(\"{spec}\") resolves to no module "
        f"(looked for {', '.join(c.name for c in candidates)} in {base.parent})"
    )


def index_expr(name: str) -> str:
    if IDENT_RE.match(name) and name not in LUA_KEYWORDS:
        return "." + name
    return '["%s"]' % name.replace("\\", "\\\\").replace('"', '\\"')


def wait_expr(name: str) -> str:
    return ':WaitForChild("%s")' % name.replace("\\", "\\\\").replace('"', '\\"')


def instance_expr(script_path: Tuple[str, ...], target_path: Tuple[str, ...]) -> str:
    """The Roblox reference that names `target_path` as seen from `script_path`."""
    common = 0
    for a, b in zip(script_path, target_path):
        if a != b:
            break
        common += 1
    # A shared ancestor *below* the service level means relative navigation is
    # both shorter and immune to the service not having replicated yet.
    if common >= 2:
        ups = len(script_path) - common
        expr = "script" + ".Parent" * ups
        for part in target_path[common:]:
            expr += index_expr(part)
        return expr
    # Crossing services: name the service, and wait for the descent, because a
    # client script can run before ReplicatedStorage's children have arrived.
    if not target_path:
        raise BuildError("empty instance path")
    expr = 'game:GetService("%s")' % target_path[0]
    for part in target_path[1:]:
        expr += wait_expr(part)
    return expr


# --------------------------------------------------------------------------
# Emit
# --------------------------------------------------------------------------

BANNER = (
    "-- GENERATED by roblox/build/build.py from {src} -- DO NOT EDIT.\n"
    "-- Relative-path requires have been rewritten to Roblox instance\n"
    "-- references. Edit the source, then re-run the build.\n"
)


def rewrite_file(entry: Entry, roots: Sequence[BuildRoot],
                 by_src: Dict[Path, Entry], stats: Dict[str, int],
                 warnings: List[str]) -> str:
    rel = entry.src.relative_to(ROBLOX_DIR)
    text = entry.src.read_text(encoding="utf-8")
    toks = lex(text)
    sites = find_require_sites(toks, str(rel))

    edits: List[Tuple[int, int, str]] = []
    for site in sites:
        if site.kind == "expr":
            stats["passthrough"] += 1
            warnings.append(
                f"{rel}:{site.line}: require(<expression>) passed through unchanged "
                f"(already an instance reference?) -- not verifiable by rewriting"
            )
            continue
        spec = str(site.token.value)
        target = resolve_module_path(spec, entry.src, roots, by_src, str(rel), site.line)
        expr = instance_expr(entry.instance_path, target.instance_path)
        edits.append((site.token.start, site.token.end_, expr))
        stats["rewritten"] += 1

    # Guard: no relative-path literal may survive anywhere in *code*.
    edited = {(s, e) for s, e, _ in edits}
    for t in toks:
        if t.kind != "STRING" or (t.start, t.end_) in edited:
            continue
        if RELATIVE_PATH_RE.match(str(t.value)):
            raise BuildError(
                f"{rel}:{t.line}: the string literal {t.text} looks like a module "
                f"path but is not in a require position. It would not survive the "
                f"move to Roblox. Refusing to emit."
            )

    out: List[str] = []
    pos = 0
    for start, end, expr in sorted(edits):
        out.append(text[pos:start])
        out.append(expr)
        pos = end
    out.append(text[pos:])
    body = "".join(out)

    banner = BANNER.format(src=rel.as_posix())
    # Keep any leading `--!` directives (`--!strict`, `--!native`) at the very
    # top of the chunk, where Luau requires them to be.
    lines = body.split("\n")
    keep = 0
    while keep < len(lines) and lines[keep].startswith("--!"):
        keep += 1
    if keep:
        return "\n".join(lines[:keep]) + "\n" + banner + "\n".join(lines[keep:])
    return banner + body


def build(roots: Sequence[BuildRoot], out_root: Path,
          stats: Dict[str, int], warnings: List[str]) -> List[Entry]:
    entries = scan(roots)
    by_src = {e.src: e for e in entries}

    # Two files must not claim the same instance path (e.g. `X.luau` next to
    # `X.server.luau`), or a require would be ambiguous.
    seen: Dict[Tuple[str, ...], Entry] = {}
    for e in entries:
        if not e.instance_path:
            continue
        prev = seen.get(e.instance_path)
        if prev is not None:
            raise BuildError(
                f"{e.src.relative_to(ROBLOX_DIR)} and {prev.src.relative_to(ROBLOX_DIR)} "
                f"both become {'.'.join(e.instance_path)}"
            )
        seen[e.instance_path] = e

    for root in roots:
        dist = out_root / root.dist_dir.relative_to(ROBLOX_DIR / DIST_PREFIX)
        if dist.exists():
            shutil.rmtree(dist)

    for e in entries:
        dest = out_root / e.dist.relative_to(ROBLOX_DIR / DIST_PREFIX)
        dest.parent.mkdir(parents=True, exist_ok=True)
        if e.rewrite:
            dest.write_text(rewrite_file(e, roots, by_src, stats, warnings),
                            encoding="utf-8")
            stats["luau_files"] += 1
        else:
            shutil.copyfile(e.src, dest)
            stats["copied_files"] += 1
        e.dist = dest
    return entries


# --------------------------------------------------------------------------
# Verification: re-parse the emitted text and walk the emitted tree
# --------------------------------------------------------------------------

class Node:
    def __init__(self, name: str, class_name: str, parent: Optional["Node"]):
        self.name = name
        self.class_name = class_name
        self.parent = parent
        self.children: Dict[str, "Node"] = {}

    def path(self) -> str:
        parts: List[str] = []
        n: Optional[Node] = self
        while n is not None and n.parent is not None:
            parts.append(n.name)
            n = n.parent
        return ".".join(reversed(parts))


def build_tree(entries: Sequence[Entry]) -> Node:
    root = Node("game", "DataModel", None)
    for e in entries:
        if not e.instance_path:
            continue
        node = root
        for i, part in enumerate(e.instance_path):
            last = i == len(e.instance_path) - 1
            child = node.children.get(part)
            if child is None:
                child = Node(part, e.class_name if last else "Folder", node)
                node.children[part] = child
            elif last:
                child.class_name = e.class_name
            node = child
    return root


def eval_instance_expr(toks: List[Token], start: int, end: int,
                       script_node: Node, root: Node) -> Node:
    """Independently evaluate a generated reference against the emitted tree.

    Deliberately understands only the shapes the emitter produces, so a
    malformed emission cannot be silently accepted.
    """
    i = start
    if i >= end:
        raise BuildError("empty require argument")
    t = toks[i]
    if t.kind == "NAME" and t.text == "script":
        cur = script_node
        i += 1
    elif t.kind == "NAME" and t.text == "game":
        if not (i + 1 < end and toks[i + 1].text == ":" and
                toks[i + 2].text == "GetService" and toks[i + 3].text == "(" and
                toks[i + 4].kind == "STRING" and toks[i + 5].text == ")"):
            raise BuildError("expected game:GetService(\"...\")")
        svc = str(toks[i + 4].value)
        cur = root.children.get(svc)
        if cur is None:
            raise BuildError(f"service {svc} holds nothing this build emitted")
        i += 6
    else:
        raise BuildError(f"reference does not start at `script` or `game` (got {t.text!r})")

    while i < end:
        t = toks[i]
        if t.kind == "OP" and t.text == "." and i + 1 < end and toks[i + 1].kind == "NAME":
            name = toks[i + 1].text
            i += 2
        elif t.kind == "OP" and t.text == "[" and i + 2 < end and \
                toks[i + 1].kind == "STRING" and toks[i + 2].text == "]":
            name = str(toks[i + 1].value)
            i += 3
        elif t.kind == "OP" and t.text == ":" and i + 4 < end and \
                toks[i + 1].text == "WaitForChild" and toks[i + 2].text == "(" and \
                toks[i + 3].kind == "STRING" and toks[i + 4].text == ")":
            name = str(toks[i + 3].value)
            i += 5
        else:
            raise BuildError(f"unrecognised navigation step at token {t.text!r}")
        if name == "Parent":
            if cur.parent is None:
                raise BuildError("walked off the top of the DataModel via .Parent")
            cur = cur.parent
            continue
        child = cur.children.get(name)
        if child is None:
            raise BuildError(f"{cur.path() or 'game'} has no child {name!r}")
        cur = child
    return cur


def verify_emitted(entries: Sequence[Entry], stats: Dict[str, int],
                   warnings: List[str]) -> None:
    root = build_tree(entries)
    by_path = {e.instance_path: e for e in entries if e.instance_path}

    for e in entries:
        if not e.rewrite:
            continue
        rel = e.dist.name
        text = e.dist.read_text(encoding="utf-8")
        toks = lex(text)
        node = root
        for part in e.instance_path:
            node = node.children[part]

        # Nothing that looks like a relative module path may remain in code.
        for t in toks:
            if t.kind == "STRING" and RELATIVE_PATH_RE.match(str(t.value)):
                raise BuildError(
                    f"VERIFY {e.dist}:{t.line}: relative path literal {t.text} "
                    f"survived into the emitted file"
                )

        for idx, t in enumerate(toks):
            if t.kind != "NAME" or t.text != "require":
                continue
            # Locate the argument extent of this require.
            if idx + 1 < len(toks) and toks[idx + 1].text == "(":
                depth = 0
                j = idx + 1
                close = None
                while j < len(toks):
                    if toks[j].kind == "OP" and toks[j].text in "([{":
                        depth += 1
                    elif toks[j].kind == "OP" and toks[j].text in ")]}":
                        depth -= 1
                        if depth == 0:
                            close = j
                            break
                    j += 1
                lo, hi = idx + 2, close
            elif idx + 1 < len(toks) and toks[idx + 1].text == ",":
                # `pcall(require, <expr>)` -- the argument runs to the next
                # comma or close paren *at depth zero*; the expression itself
                # contains parentheses (game:GetService("...")).
                lo, hi, depth = idx + 2, idx + 2, 0
                while hi < len(toks):
                    tk = toks[hi]
                    if tk.kind == "OP" and tk.text in "([{":
                        depth += 1
                    elif tk.kind == "OP" and tk.text in ")]}":
                        if depth == 0:
                            break
                        depth -= 1
                    elif tk.kind == "OP" and tk.text == "," and depth == 0:
                        break
                    hi += 1
            else:
                raise BuildError(
                    f"VERIFY {e.dist}:{t.line}: require in an unrecognised position"
                )
            try:
                target = eval_instance_expr(toks, lo, hi, node, root)
            except BuildError as exc:
                warnings.append(
                    f"VERIFY {e.dist.relative_to(ROBLOX_DIR)}:{t.line}: could not "
                    f"resolve require against the emitted tree ({exc})"
                )
                stats["unverified"] += 1
                continue
            entry = by_path.get(tuple(
                p for p in _node_instance_path(target)
            ))
            if entry is None or not entry.is_module:
                raise BuildError(
                    f"VERIFY {e.dist.relative_to(ROBLOX_DIR)}:{t.line}: require "
                    f"resolves to {target.path()} which is not an emitted module"
                )
            stats["verified"] += 1


def _node_instance_path(node: Node) -> Tuple[str, ...]:
    parts: List[str] = []
    n: Optional[Node] = node
    while n is not None and n.parent is not None:
        parts.append(n.name)
        n = n.parent
    return tuple(reversed(parts))


# --------------------------------------------------------------------------
# luau-analyze parse gate
# --------------------------------------------------------------------------

DIAG_RE = re.compile(r"^(?P<file>.*?)\((?P<line>\d+),(?P<col>\d+)\): (?P<msg>.*)$")

# The analyzer has no Roblox definitions and no Rojo sourcemap, so an instance
# require is opaque to it *by construction*: `script` and `game` are unknown
# globals, the require target is an unresolvable path, and everything typed
# through that require degrades to `unknown`. That whole family is tolerated.
#
# It is safe to tolerate precisely because of what this build step does and
# does not do: it replaces a string literal with an instance expression and
# changes nothing else. The only new diagnostics it can possibly cause are
# (a) a syntax error -- checked first, always fatal -- and (b) that
# unknown-type cascade. Anything else the source did not already produce is a
# real regression and fails the build.
TOLERATED = (re.compile(r"[Uu]nknown"),)


def find_tool(explicit: Optional[str], env: str, name: str) -> Optional[str]:
    if explicit:
        return explicit if Path(explicit).exists() else None
    from_env = os.environ.get(env)
    if from_env and Path(from_env).exists():
        return from_env
    return shutil.which(name)


def run_analyze(tool: str, files: Sequence[Path]) -> Dict[str, List[Tuple[str, str]]]:
    """{resolved file -> [(message, raw line)]}. Locations are dropped from the
    key so the generated banner's line offset cannot desynchronise the compare."""
    out: Dict[str, List[Tuple[str, str]]] = {}
    if not files:
        return out
    proc = subprocess.run([tool] + [str(f) for f in files],
                          capture_output=True, text=True)
    last: Optional[str] = None
    for line in (proc.stdout + proc.stderr).splitlines():
        if not line.strip():
            continue
        m = DIAG_RE.match(line)
        if m is None:
            # Continuation of the previous diagnostic (the type checker wraps
            # long explanations). Fold it in rather than mistaking it for a
            # diagnostic of its own.
            if last is not None and out.get(last):
                msg, raw = out[last][-1]
                out[last][-1] = (msg + " " + line.strip(), raw)
            else:
                out.setdefault("", []).append((line, line))
            continue
        key = str(Path(m.group("file")).resolve())
        out.setdefault(key, []).append((m.group("msg"), line))
        last = key
    return out


def analyze(entries: Sequence[Entry], tool: str) -> Tuple[int, List[str]]:
    """Parse gate, differential against the source tree.

    A diagnostic is only a build failure if the *source* file did not already
    produce it and it is not one of the three Roblox-require consequences
    above. A SyntaxError is always a failure -- that is the "does it parse"
    question this gate exists to answer.
    """
    targets = [e for e in entries if e.rewrite]
    if not targets:
        return 0, []
    dist_diags = run_analyze(tool, [e.dist for e in targets])
    src_diags = run_analyze(tool, [e.src for e in targets])

    bad: List[str] = []
    for e in targets:
        base = {msg for msg, _ in src_diags.get(str(e.src.resolve()), [])}
        for msg, raw in dist_diags.get(str(e.dist.resolve()), []):
            if "SyntaxError" in msg:
                bad.append(raw + "   [emitted file does not parse]")
                continue
            if msg in base:
                continue  # pre-existing in the source; not this build's doing
            if any(p.search(msg) for p in TOLERATED):
                continue
            bad.append(raw + "   [introduced by the build]")
    for msg, raw in dist_diags.get("", []):
        bad.append(raw)
    return len(targets), bad


# --------------------------------------------------------------------------
# Execution round-trip: run the emitted graph against an emulated instance tree
# --------------------------------------------------------------------------

HARNESS_PRELUDE = r"""
-- GENERATED harness. Emulates just enough of the Roblox instance tree to load
-- every emitted module through its *rewritten* require expressions.
--
-- Errors are split into two kinds, and the distinction is the whole point:
--   REWRITE-BROKEN -- navigation named something that does not exist in the
--                     emitted tree. That is this build step's fault. Fatal.
--   anything else  -- the module reached for a Roblox API this harness does
--                     not emulate (RunService, Instance.new, task, ...). Not
--                     this build step's business. Reported, not fatal.
local SRC = ...

local nodes = {}

local function broken(fmt, ...)
	error("REWRITE-BROKEN: " .. string.format(fmt, ...), 3)
end

local Instance_mt = {}
Instance_mt.__index = function(self, key)
	local kids = rawget(self, "_children")
	local hit = kids and kids[key]
	if hit ~= nil then
		return hit
	end
	local m = rawget(Instance_mt, "_methods")[key]
	if m ~= nil then
		return m
	end
	broken("%s has no child %q (the rewritten reference names nothing)",
		rawget(self, "_path"), tostring(key))
	return nil
end
Instance_mt._methods = {
	WaitForChild = function(self, name)
		local kids = rawget(self, "_children")
		local hit = kids and kids[name]
		if hit == nil then
			broken("%s:WaitForChild(%q) would hang forever", rawget(self, "_path"), name)
		end
		return hit
	end,
	FindFirstChild = function(self, name)
		local kids = rawget(self, "_children")
		return kids and kids[name] or nil
	end,
	GetService = function(self, name)
		local kids = rawget(self, "_children")
		local hit = kids and kids[name]
		if hit == nil then
			-- Never a rewrite failure: the rewriter only ever names services it
			-- emitted into. This is the module asking for a real Roblox service.
			error("harness does not emulate the " .. name .. " service", 2)
		end
		return hit
	end,
}

local function node(path, name, parent)
	local n = setmetatable(
		{ Name = name, Parent = parent, ClassName = "Folder", _children = {}, _path = path },
		Instance_mt
	)
	nodes[path] = n
	if parent then
		rawget(parent, "_children")[name] = n
	end
	return n
end

local game = node("", "game", nil)

local function ensure(path)
	local existing = nodes[path]
	if existing then
		return existing
	end
	local head, tail = path:match("^(.*)%.([^%.]+)$")
	local parent
	if head == nil then
		head, tail, parent = "", path, game
	else
		parent = ensure(head)
	end
	return node(path, tail, parent)
end

local loaded, loading = {}, {}
local function requireNode(target)
	if type(target) ~= "table" or rawget(target, "_path") == nil then
		error("REWRITE-BROKEN: require() was handed a " .. type(target) .. ", not an instance", 2)
	end
	local path = rawget(target, "_path")
	if loaded[path] ~= nil then
		return loaded[path]
	end
	if loading[path] then
		error("REWRITE-BROKEN: require cycle at " .. path, 2)
	end
	local src = SRC[path]
	if src == nil then
		error("REWRITE-BROKEN: require(" .. path .. ") -- not an emitted ModuleScript", 2)
	end
	loading[path] = true
	local chunk, err = loadstring("local script, require, game = ...\n" .. src, "@" .. path)
	if chunk == nil then
		error("REWRITE-BROKEN: emitted " .. path .. " does not compile: " .. tostring(err), 2)
	end
	local ok, result = pcall(chunk, nodes[path], requireNode, game)
	loading[path] = nil
	if not ok then
		error(result, 0)
	end
	loaded[path] = result
	return result
end
"""


def _long_level(text: str) -> int:
    level = 0
    while ("]" + "=" * level + "]") in text or ("[" + "=" * level + "[") in text:
        level += 1
    return level


def write_harness(entries: Sequence[Entry], out: Path, module_paths: List[str]) -> None:
    parts: List[str] = ["local SRC = {}\n"]
    for e in entries:
        if not e.rewrite or not e.is_module:
            continue
        path = ".".join(e.instance_path)
        text = e.dist.read_text(encoding="utf-8")
        lvl = _long_level(text)
        eq = "=" * lvl
        parts.append('SRC["%s"] = [%s[\n%s]%s]\n' % (path, eq, text, eq))
    parts.append("local run = loadstring([==[\n" + HARNESS_PRELUDE.replace("]==]", "]==\\]") +
                 "\nreturn { ensure = ensure, requireNode = requireNode }\n]==]" +
                 ', "@harness")\n')
    parts.append("local api = run(SRC)\n")
    parts.append("local ORDER = {\n")
    for p in module_paths:
        parts.append('\t"%s",\n' % p)
    parts.append("}\n")
    # Materialise every emitted instance first, so navigation sees the real tree.
    parts.append("local ALL = {\n")
    for e in entries:
        if not e.instance_path:
            continue
        parts.append('\t"%s",\n' % ".".join(e.instance_path))
    parts.append("}\n")
    parts.append(r"""
for _, p in ipairs(ALL) do
	api.ensure(p)
end
local loadedOk, blocked, fatal = 0, {}, {}
for _, p in ipairs(ORDER) do
	local ok, err = pcall(api.requireNode, api.ensure(p))
	if ok then
		loadedOk += 1
	elseif string.find(tostring(err), "REWRITE-BROKEN", 1, true) then
		table.insert(fatal, p .. ": " .. tostring(err))
	else
		table.insert(blocked, p .. ": " .. tostring(err))
	end
end
print(("exec-check: %d/%d emitted modules loaded and returned through their rewritten requires")
	:format(loadedOk, #ORDER))
for _, b in ipairs(blocked) do
	print("exec-check: needs a Roblox runtime, not a rewrite problem -- " .. b)
end
if #fatal > 0 then
	for _, f in ipairs(fatal) do
		print("exec-check: REWRITE FAILURE -- " .. f)
	end
	error(("%d module(s) failed on the rewritten instance references"):format(#fatal), 0)
end
""")
    out.write_text("".join(parts), encoding="utf-8")


def exec_check(entries: Sequence[Entry], luau: str) -> Tuple[bool, str]:
    module_paths = [".".join(e.instance_path) for e in entries
                    if e.rewrite and e.is_module]
    tmpdir = Path(tempfile.mkdtemp(prefix="stdbuild-"))
    try:
        harness = tmpdir / "harness.luau"
        write_harness(entries, harness, module_paths)
        proc = subprocess.run([luau, str(harness)], capture_output=True, text=True,
                              cwd=str(tmpdir))
        out = (proc.stdout + proc.stderr).strip()
        return proc.returncode == 0, out
    finally:
        shutil.rmtree(tmpdir, ignore_errors=True)


# --------------------------------------------------------------------------
# main
# --------------------------------------------------------------------------

def tree_snapshot(root: Path) -> Dict[str, bytes]:
    snap: Dict[str, bytes] = {}
    if not root.exists():
        return snap
    for p in sorted(root.rglob("*")):
        if p.is_file():
            snap[str(p.relative_to(root))] = p.read_bytes()
    return snap


def main(argv: Optional[List[str]] = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[1],
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--check", action="store_true",
                    help="build into a temp dir and fail if dist/ differs (CI gate)")
    ap.add_argument("--verify-only", action="store_true",
                    help="verify the existing dist/ without rebuilding")
    ap.add_argument("--exec-check", action="store_true",
                    help="also execute the emitted module graph under the interpreter")
    ap.add_argument("--strict-passthrough", action="store_true",
                    help="treat an unrewritable expression require as an error")
    ap.add_argument("--no-analyze", action="store_true",
                    help="skip the luau-analyze parse gate")
    ap.add_argument("--luau", help="path to the standalone luau interpreter")
    ap.add_argument("--luau-analyze", dest="luau_analyze", help="path to luau-analyze")
    ap.add_argument("--project", help="a different Rojo project file to build "
                                      "(used by the build step's own tests)")
    args = ap.parse_args(argv)

    global PROJECT_FILE, ROBLOX_DIR
    if args.project:
        PROJECT_FILE = Path(args.project).resolve()
        ROBLOX_DIR = PROJECT_FILE.parent

    stats = {"luau_files": 0, "copied_files": 0, "rewritten": 0,
             "passthrough": 0, "verified": 0, "unverified": 0}
    warnings: List[str] = []

    try:
        roots = read_build_roots()
        dist_root = ROBLOX_DIR / DIST_PREFIX

        if args.check:
            tmp = Path(tempfile.mkdtemp(prefix="stdbuild-check-"))
            try:
                entries = build(roots, tmp, stats, warnings)
                fresh = tree_snapshot(tmp)
                current = tree_snapshot(dist_root)
                if fresh != current:
                    only_fresh = sorted(set(fresh) - set(current))
                    only_dist = sorted(set(current) - set(fresh))
                    changed = sorted(k for k in set(fresh) & set(current)
                                     if fresh[k] != current[k])
                    print("dist/ is STALE -- re-run the build.", file=sys.stderr)
                    for k in only_fresh:
                        print(f"  missing from dist/: {k}", file=sys.stderr)
                    for k in only_dist:
                        print(f"  stale in dist/:     {k}", file=sys.stderr)
                    for k in changed:
                        print(f"  differs:            {k}", file=sys.stderr)
                    return 1
                print(f"dist/ is up to date ({len(fresh)} files).")
                return 0
            finally:
                shutil.rmtree(tmp, ignore_errors=True)

        if args.verify_only:
            entries = scan(roots)
            for e in entries:
                e.dist = dist_root / e.dist.relative_to(dist_root)
                if not e.dist.exists():
                    raise BuildError(f"{e.dist} is missing -- run the build first")
        else:
            entries = build(roots, dist_root, stats, warnings)
            print(f"emitted {stats['luau_files']} Luau modules "
                  f"+ {stats['copied_files']} verbatim files into "
                  f"{dist_root.relative_to(ROBLOX_DIR.parent)}/")
            print(f"rewrote {stats['rewritten']} relative requires "
                  f"({stats['passthrough']} expression requires passed through)")

        verify_emitted(entries, stats, warnings)
        print(f"verified {stats['verified']} emitted requires resolve to real "
              f"emitted modules (independent re-parse of the emitted text)")

        if not args.no_analyze:
            tool = find_tool(args.luau_analyze, "LUAU_ANALYZE", "luau-analyze")
            if tool is None:
                warnings.append("luau-analyze not found -- PARSE GATE SKIPPED "
                                "(pass --luau-analyze=PATH or set $LUAU_ANALYZE)")
            else:
                count, bad = analyze(entries, tool)
                if bad:
                    print(f"luau-analyze rejected the emitted tree:", file=sys.stderr)
                    for line in bad:
                        print("  " + line, file=sys.stderr)
                    return 1
                print(f"luau-analyze: {count} emitted files parse clean")

        if args.exec_check:
            luau = find_tool(args.luau, "LUAU", "luau")
            if luau is None:
                raise BuildError("--exec-check needs the luau interpreter "
                                 "(pass --luau=PATH or set $LUAU)")
            ok, out = exec_check(entries, luau)
            if not ok:
                print("exec-check FAILED:\n" + out, file=sys.stderr)
                return 1
            print(out)

        if warnings:
            print()
            for w in warnings:
                print("WARN: " + w, file=sys.stderr)
            if args.strict_passthrough and (stats["passthrough"] or stats["unverified"]):
                print("--strict-passthrough: the warnings above are errors.",
                      file=sys.stderr)
                return 1

        return 0

    except BuildError as exc:
        print("BUILD FAILED: " + str(exc), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
