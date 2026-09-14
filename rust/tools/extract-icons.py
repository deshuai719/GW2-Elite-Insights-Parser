#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""提取 ParserIcons.cs 的图标表(prof/target/minion)到 rust/content/icons.json。

来源:
- GW2EI.Library/GW2EI.Services/GW2EIEvtcParser/ParserHelpers/Images/ParserIcons.cs
- GW2EI.Library/GW2EI.Services/GW2EIEvtcParser/ParserHelpers/IDs/SpeciesIDs.cs
  (TargetID/MinionID 枚举数值 + 文件内 const int)

输出:
{
  "prof":   { "<Spec name>": url },          # BaseResProfIcons
  "target": { "<numeric id>": url },         # TargetNPCIcons
  "minion": { "<numeric id>": url },         # MinionNPCIcons
  "fallback": { "prof": url, "gadget": url, "npcUnknown": url, "npcGeneric": url }
}
用法: python rust/tools/extract-icons.py <C# 仓库根>;产物 rust/content/icons.json。
校验失败(枚举解析不完整/期望值不符)时非零退出。
"""
import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]  # rust/ 目录(内容资产在其 content/)
DEFAULT_SRC = REPO.parent  # fork 仓库根(与 rust/ 同级)
SRC = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_SRC
PARSER = SRC / "GW2EI.Library" / "GW2EI.Services" / "GW2EIEvtcParser"
ICON_SRC = PARSER / "ParserHelpers" / "Images" / "ParserIcons.cs"
SPECIES_SRC = PARSER / "ParserHelpers" / "IDs" / "SpeciesIDs.cs"
OUT = REPO / "content" / "icons.json"

strip_comment = lambda s: re.sub(r"//.*$", "", s).strip()

# ---------------- 1. SpeciesIDs 枚举数值 ----------------
text = SPECIES_SRC.read_text(encoding="utf-8")

# 文件级 const int(含 private const int ... = ...;)
consts = {}
for m in re.finditer(r"const\s+int\s+(\w+)\s*=\s*([^;,]+)(?:,|;)", strip_comment(text)):
    consts[m.group(1)] = m.group(2).strip()

# 全部 enum 段;成员表达式或自增(哨兵)
enums = {}
for m in re.finditer(r"enum\s+(\w+)\s*(?::\s*\w+)?\s*\{(.*?)\n\s*\};", text, flags=re.S):
    name = m.group(1)
    members: dict[str, str] = {}
    for line in m.group(2).splitlines():
        s = strip_comment(line)
        if not s:
            continue
        s = s.rstrip(",")
        mm = re.match(r"(\w+)\s*(?:=\s*(.*?))?$", s)
        if not mm:
            continue
        nm, expr = mm.group(1), mm.group(2)
        members[nm] = expr.strip() if expr else "__AUTO__"
    enums[name] = members

# 统一解析空间:file consts + 全部枚举成员(限定名与裸名同指原始表达式)
universe = {f"SpeciesIDs.{k}": v for k, v in consts.items()}
for k, v in consts.items():
    universe.setdefault(k, v)
for en, members in enums.items():
    for k, v in members.items():
        universe[f"{en}.{k}"] = v
        universe.setdefault(k, (en, k))

# 先解析自增成员(源码序前一值+1;仅允许在显式值的末尾链)
TOKEN = re.compile(r"0[xX][0-9a-fA-F]+|\d+|\w+(?:\.\w+)?|[()|&~+\-]")
resolved: dict[str, int] = {}


def lookup(tk: str, scope: str):
    """tk 解析:限定名直接查;裸名先按作用域枚举查,再退回全局(可能跨枚举)。"""
    if "." in tk:
        return universe.get(tk)
    ent = universe.get(f"{scope}.{tk}")
    if ent is not None:
        return ent
    return universe.get(tk)


def eval_expr(expr: str, scope: str, depth: int = 0) -> int:
    if depth > 60:
        raise SystemExit(f"circular ref at {expr}")
    expr = strip_comment(expr)
    tokens = TOKEN.findall(expr)
    if "".join(tokens) != expr:
        raise SystemExit(f"unparsed expr tokens: {expr!r}")
    out = []
    for tk in tokens:
        if re.fullmatch(r"0[xX][0-9a-fA-F]+", tk):
            out.append(str(int(tk, 16)))
        elif re.fullmatch(r"\d+", tk):
            out.append(tk)
        elif tk in ("|", "&", "~", "+", "-", "(", ")"):
            out.append(tk)
        elif re.fullmatch(r"\w+(?:\.\w+)?", tk):
            key = tk if "." in tk else f"{scope}.{tk}"
            target = resolved.get(key)
            if target is None:
                if tk in ("int.MaxValue", "int.MinValue"):
                    target = 0x7FFFFFFF if tk == "int.MaxValue" else -0x80000000
                else:
                    src = lookup(tk, scope)
                    if src is None:
                        raise SystemExit(f"unresolved identifier {tk!r} in {expr!r}")
                    target = eval_expr(src, scope, depth + 1)
                resolved[key] = target
            out.append(str(target))
        else:
            raise SystemExit(f"bad token {tk!r} in {expr!r}")
    py = " ".join(out).replace("|", " | ").replace("&", " & ").replace("~", " ~ ")
    try:
        return eval(py)
    except Exception as e:  # noqa: BLE001
        raise SystemExit(f"eval failed {py!r}: {e}") from e


def resolve_enum(en: str) -> dict[str, int]:
    members = enums[en]
    vals: dict[str, int] = {}
    prev = 0
    for nm, expr in members.items():
        if expr == "__AUTO__":
            val = prev + 1
        else:
            val = eval_expr(expr, en)
        prev = val
        vals[nm] = val
        resolved[f"{en}.{nm}"] = val
    return vals


TARGET = resolve_enum("TargetID")
MINION = resolve_enum("MinionID")

expect = {
    "TargetID": {"WorldVersusWorld": 1, "Instance": 2, "DummyTarget": -1, "Environment": -36, "Unknown": 0x7FFFFFFF},
    "MinionID": {
        "IllusionaryAvenger": 15188, "IllusionarySharpShooter": 26152, "JuvenileBrownBear": 4426,
        "FrostSpirit": 6369, "StoneSpirit": 6370, "SpiritOfNatureRenewal": 6649,
        "JuvenilePolarBear": 7926, "JuvenileSiegeTurtle": 24796,
    },
}
for en, checks in expect.items():
    table = {"TargetID": TARGET, "MinionID": MINION}[en]
    for nm, val in checks.items():
        got = table.get(nm)
        if got != val:
            raise SystemExit(f"sanity failed: {en}.{nm} = {got}, expected {val}")

# ---------------- 2. ParserIcons 图标字典 ----------------
src = ICON_SRC.read_text(encoding="utf-8")
consts_str = {}
for m in re.finditer(r"const\s+string\s+(\w+)\s*=\s*\"([^\"]*)\"", src):
    consts_str[m.group(1)] = m.group(2)


def parse_icon_dict(body: str) -> dict:
    out = {}
    for line in body.splitlines():
        s = strip_comment(line)
        mm = re.match(r"\{\s*(?:Spec|TargetID|MinionID)\.(\w+),\s*(?:\"([^\"]*)\"|(\w+))\s*\}", s)
        if not mm:
            continue
        nm, lit, ref = mm.group(1), mm.group(2), mm.group(3)
        if lit is not None:
            out[nm] = lit
        elif ref in consts_str:
            out[nm] = consts_str[ref]
        else:
            raise SystemExit(f"unresolved icon const {ref!r} for {nm}")
    return out


def extract_block(dict_name: str, text: str) -> dict:
    mm = re.search(
        rf"readonly\s+static\s+IReadOnlyDictionary<\w+,\s*string>\s+{dict_name}\s*=\s*new\s+Dictionary<\w+,\s*string>\s*\(\s*\)\s*\{{(.*?)\n\s*\}};",
        text,
        flags=re.S,
    )
    if not mm:
        raise SystemExit(f"icon dict {dict_name} not found")
    return parse_icon_dict(mm.group(1))


prof = extract_block("BaseResProfIcons", src)
target_icons = extract_block("TargetNPCIcons", src)
minion_icons = extract_block("MinionNPCIcons", src)
fallback = {
    "prof": consts_str.get("UnknownProfessionIcon", ""),
    "gadget": consts_str.get("GenericGadgetIcon", ""),
    "npcUnknown": consts_str.get("UnknownNPCIcon", ""),
    "npcGeneric": consts_str.get("GenericEnemyIcon", ""),
}
for k, v in fallback.items():
    if not v:
        raise SystemExit(f"missing fallback {k}")

out = {
    "prof": prof,
    "target": {str(TARGET[nm]): url for nm, url in target_icons.items()},
    "minion": {str(MINION[nm]): url for nm, url in minion_icons.items()},
    "fallback": fallback,
}
# 覆盖校验:WvW 相关物种(伪 target + 宠物/精魂/幻象)
for sid in ("1", "4426", "7926", "24796", "6369", "6370", "6649", "15188", "26152"):
    if sid not in out["target"] and sid not in out["minion"]:
        raise SystemExit(f"icon coverage missing id {sid}")

# ---------------- 3. IsUniquePerTimeFrame 的 Ranger 幼宠集合 ----------------
# (Minions.cs:33-36 的 RangerHelper.IsJuvenilePet;union 于 RangerHelper.cs)
ranger_src = (PARSER / "EIData" / "ProfHelpers" / "Ranger" / "RangerHelper.cs").read_text(
    encoding="utf-8"
)
pet_sets = {}
pet_union = {}
for m in re.finditer(
    r"HashSet<int>\s+(\w+)\s*=\s*(?:new\s+HashSet<int>\s*)?\{\s*(.*?)\s*\}|HashSet<int>\s+(\w+)\s*=\s*\[\s*(.*?)\s*\];",
    ranger_src,
    flags=re.S,
):
    name = m.group(1) or m.group(3)
    body = m.group(2) or m.group(4) or ""
    pet_sets[name] = re.findall(r"\(int\)MinionID\.(\w+)", body)
    # 语句其余部分(到 ';')的 .Union(SetName) 引用
    tail = ranger_src[m.end():]
    semi = tail.find(";")
    tail = tail[:semi] if semi >= 0 else tail
    pet_union[name] = re.findall(r"\.Union\((\w+)\)", tail)


def resolve_pets(name: str, seen: frozenset) -> set:
    out = {MINION[nm] for nm in pet_sets.get(name, []) if nm in MINION}
    for sub in pet_union.get(name, []):
        if sub in pet_sets and sub not in seen:
            out |= resolve_pets(sub, seen | {sub})
    return out


out["uniqueMinions"] = {
    "rangerJuvenile": sorted(resolve_pets("JuvenilePetIDs", frozenset())),
    "rangerKnown": sorted(
        resolve_pets("JuvenilePetIDs", frozenset()) | resolve_pets("SpiritIDs", frozenset())
    ),
}
# ---------------- 4. Mesmer 已知 minion(幻象/分身;known-minion 门控)----
MESMER_SRC = (PARSER / "EIData" / "ProfHelpers" / "Mesmer" / "MesmerHelper.cs").read_text(
    encoding="utf-8"
)
mesmer_names = []
for set_name in ("_clones", "_phantasms"):
    mm = re.search(rf"{set_name}\s*=\s*\[(.*?)\];", MESMER_SRC, flags=re.S)
    if not mm:
        raise SystemExit(f"mesmer set {set_name} not found")
    for nm in re.findall(r"\(int\)MinionID\.(\w+)", mm.group(1)):
        if nm not in MINION:
            raise SystemExit(f"mesmer minion name {nm} not in MinionID")
        mesmer_names.append(MINION[nm])
out["uniqueMinions"]["mesmerKnown"] = sorted(mesmer_names)
OUT.write_text(json.dumps(out, ensure_ascii=False, indent=0), encoding="utf-8")
print(
    f"icons.json written: {len(out['prof'])} prof, "
    f"{len(out['target'])} target, {len(out['minion'])} minion"
)
