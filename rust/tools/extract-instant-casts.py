#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""抽取 EI ProfHelpers 的 InstantCastFinder 表为 JSON（P3b 最小子集）。

输入:GW2-Elite-Insights-Parser fork 的 EIData/ProfHelpers(相对本仓库根)。
输出:rust/content/instant-cast-finders.json(入库,供 gw2ei-json 合成引擎)。

范围(design.md/ei-instantcast.md §5 最小子集):
- ProfHelper.cs 两个通用表(无条件注册)
- 15 个 Helper 文件:BaseSpec(Guardian/Engineer/Mesmer/Ranger/Revenant/
  Necromancer/Warrior) + Spec(Holosmith/Scourge/Druid/Troubadour/Paragon/
  Vindicator/Dragonhunter/Harbinger) 涉及的 Helper.InstantCastFinder 表。
- EngineerKitFinder 手工移植(有状态,不在表中表达)—— 见 Rust 引擎。

常量解析:SkillIDs.cs(long)、EffectGUIDs.cs(readonly GUID hex)、
GW2Builds.cs(ulong)、SpeciesIDs.cs 的 MinionID 枚举(用于 (int)MinionID.X)。
检查器:机械 fluent 方法调用按名保留参数(structured),lambda(UsingChecker/
UsingEnable 带 `=>`)保留原文并登记 custom;无法机械翻译的登记 unresolved
(不硬造)。

用法:python rust/tools/extract-instant-casts.py [--src C:/path/to/fork]
输出统计到 stdout;JSON 写 rust/content/instant-cast-finders.json。
"""

import argparse
import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]  # rust/ 目录(内容资产在其 content/)
DEFAULT_SRC = REPO.parent  # fork 仓库根(与 rust/ 同级)

# C# ParserHelper.ServerDelayConstant —— lambda 翻译时省略的窗口/eps 参数
# 取 C# 签名默认值(CombatDataHelpers.FindRelatedEvents 等),不能产出 0:
# 引擎窗口判定 |t - ev| < eps 在 eps=0 时恒不命中。
SERVER_DELAY_CONST = 10

# Helper 文件 → 注册组(BaseSpec 组取 Spec.BaseSpec() 语义 / Spec 组按 Spec)
HELPERS = {
    # (组类型, Spec 名) → 相对 EIData/ProfHelpers 的文件
    "base": {
        "Guardian": "Guardian/GuardianHelper.cs",
        "Engineer": "Engineer/EngineerHelper.cs",
        "Mesmer": "Mesmer/MesmerHelper.cs",
        "Ranger": "Ranger/RangerHelper.cs",
        "Revenant": "Revenant/RevenantHelper.cs",
        "Necromancer": "Necromancer/NecromancerHelper.cs",
        "Warrior": "Warrior/WarriorHelper.cs",
        "Elementalist": "Elementalist/ElementalistHelper.cs",
    },
    "spec": {
        "Holosmith": "Engineer/HolosmithHelper.cs",
        "Scourge": "Necromancer/ScourgeHelper.cs",
        "Druid": "Ranger/DruidHelper.cs",
        "Troubadour": "Mesmer/TroubadourHelper.cs",
        "Paragon": "Warrior/ParagonHelper.cs",
        "Vindicator": "Revenant/VindicatorHelper.cs",
        "Dragonhunter": "Guardian/DragonhunterHelper.cs",
        "Harbinger": "Necromancer/HarbingerHelper.cs",
        "Evoker": "Elementalist/EvokerHelper.cs",
        "Chronomancer": "Mesmer/ChronomancerHelper.cs",
        "Scrapper": "Engineer/ScrapperHelper.cs",
        "Berserker": "Warrior/BerserkerHelper.cs",
        "Amalgam": "Engineer/AmalgamHelper.cs",
    },
}

# finder 类名白名单(全部继承 InstantCastFinder;EngineerKitFinder 是
# WeaponSwapCastFinder 子类且带状态 → 排除,引擎手工处理)
FINDER_CLASSES = {
    "BuffGainCastFinder", "BuffLossCastFinder", "BuffGiveCastFinder",
    "BuffExtendCastFinder", "DamageCastFinder", "BreakbarDamageCastFinder",
    "EffectCastFinder", "EffectCastFinderByDst", "MinionSpawnCastFinder",
    "MinionCommandCastFinder", "MinionCastCastFinder", "MissileCastFinder",
    "MarkerCastFinder", "WeaponSwapCastFinder", "EXTHealingCastFinder",
    "EXTBarrierCastFinder",
}

# ---- 常量表解析 ----

def parse_skill_ids(text):
    out = {}
    for m in re.finditer(r"public\s+const\s+long\s+(\w+)\s*=\s*(-?\d+)\s*;", text):
        out[m.group(1)] = int(m.group(2))
    # SkillItem 名(个别 const long 为十六进制?上面正则已覆盖十进制;十六进制兜底)
    for m in re.finditer(r"public\s+const\s+long\s+(\w+)\s*=\s*(0x[0-9a-fA-F]+)\s*;", text):
        out[m.group(1)] = int(m.group(2), 16)
    return out


def parse_guid_consts(text):
    out = {}
    for m in re.finditer(
        r"public\s+static\s+readonly\s+GUID\s+(\w+)\s*=\s*new\(\s*\"([0-9A-Fa-f]{32})\"\s*\)\s*;",
        text,
    ):
        out[m.group(1)] = m.group(2).upper()
    return out


def parse_gw2_builds(text):
    out = {}
    for m in re.finditer(r"public\s+const\s+ulong\s+(\w+)\s*=\s*(\d+)\s*;", text):
        out[m.group(1)] = int(m.group(2))
    # 特殊值 ulong.MinValue/MaxValue(StartOfLife/EndOfLife)
    for m in re.finditer(r"public\s+const\s+ulong\s+(\w+)\s*=\s*ulong\.(\w+)\s*;", text):
        out[m.group(1)] = 0 if m.group(2) == "MinValue" else (1 << 64) - 1
    return out


def parse_minion_id_enum(text):
    """SpeciesIDs.cs 的 `public enum MinionID : int { ... }`(显式/隐式值;
    值可为别名 SpeciesIDs.X → 先解文件内 const int 常量)。"""
    consts = {}
    for m in re.finditer(r"const\s+int\s+(\w+)\s*=\s*(-?\d+)\s*;", text):
        consts[m.group(1)] = int(m.group(2))
    m = re.search(r"public\s+enum\s+MinionID\s*:\s*int\s*\{(.*?)\n\s*\}", text, re.S)
    if not m:
        return {}
    out = {}
    cur = -1
    for line in m.group(1).splitlines():
        line = line.split("//")[0].strip()
        if not line:
            continue
        cm = re.match(r"(\w+)\s*(?:=\s*(.*?)\s*)?,", line)
        if not cm:
            continue
        name = cm.group(1)
        rhs = cm.group(2)
        if rhs is not None:
            rhs = rhs.strip()
            if rhs.isdigit() or rhs.startswith("-"):
                cur = int(rhs)
            elif rhs.startswith("SpeciesIDs."):
                key = rhs[len("SpeciesIDs."):]
                if key not in consts:
                    return {}
                cur = consts[key]
            else:
                return {}
        else:
            cur += 1
        out[name] = cur
    return out


# ---- 表与条目切分 ----

def strip_comments(text):
    """删除 // 与 /* */ 注释(字符串内不处理 —— 常量表/表段无字符串字面量)。"""
    out = []
    i, n = 0, len(text)
    while i < n:
        if text[i] == "/" and i + 1 < n and text[i + 1] == "/":
            j = text.find("\n", i)
            i = n if j < 0 else j
        elif text[i] == "/" and i + 1 < n and text[i + 1] == "*":
            j = text.find("*/", i + 2)
            i = n if j < 0 else j + 2
        else:
            out.append(text[i])
            i += 1
    return "".join(out)




def _read_balanced(text, start):
    """从 text[start] 起配平括号/方括号/花括号,返回 (inner, end_excl)。"""
    depth = 0
    i = start
    while i < len(text):
        ch = text[i]
        if ch in "([{":
            depth += 1
        elif ch in ")]}":
            depth -= 1
            if depth == 0:
                return text[start + 1 : i], i + 1
        i += 1
    return text[start + 1 :], len(text)


def collect_local_sets(text, tables):
    """文件内所有集合字段(name → 成员数值列表)。支持:
       HashSet<int> X = new HashSet<int> { ... }.Union(Y)...;
       HashSet<int> X = [ ... ]; / List<int>/int[] 同
       成员 `(int)MinionID.N` 或裸整数;Union 目标递归展开。
    """
    fields = {}
    pat = re.compile(
        r"(?:private|internal|public)?\s*static\s+readonly\s+"
        r"(?:HashSet<int>|List<int>|int\[\])\s+(\w+)\s*="
    )
    for m in pat.finditer(text):
        name = m.group(1)
        i = m.end()
        while i < len(text) and text[i].isspace():
            i += 1
        body = None
        if i < len(text) and text[i] == "[":
            body, i = _read_balanced(text, i)
        elif text.startswith("new ", i):
            j = text.find("{", i)
            if j < 0:
                continue
            body, i = _read_balanced(text, j)
        else:
            continue
        # 定义尾 = 下一个 ';'(Union 链后;不含后续字段)
        j = text.find(";", i)
        tail = text[i:] if j < 0 else text[i:j]
        fields[name] = (body, tail)  # (成员体, 后续含 .Union 链)

    memo = {}

    def expand(name):
        if name in memo:
            return memo[name]
        if name not in fields:
            return None
        memo[name] = None  # 防环
        body, tail = fields[name]
        vals = []
        # 成员: (int)MinionID.X 或裸整数
        for mm in re.finditer(r"\(int\)MinionID\.(\w+)|(-?\d+)", body):
            if mm.group(1):
                v = tables["minion"].get(mm.group(1))
                if v is None:
                    return None
                vals.append(v)
            else:
                vals.append(int(mm.group(2)))
        # Union 链
        for um in re.finditer(r"\.Union\((\w+)\)", tail):
            sub = expand(um.group(1))
            if sub is None:
                return None
            vals.extend(sub)
        memo[name] = vals
        return vals

    return {n: expand(n) for n in fields if expand(n) is not None}


def find_table(text, field):
    """定位 `static readonly List<InstantCastFinder> <field> = [` 的 [ 段。"""
    pat = re.compile(r"List<InstantCastFinder>\s+" + re.escape(field) + r"\s*=\s*\[")
    m = pat.search(text)
    if not m:
        return None
    start = m.end() - 1  # 指向 '['
    depth = 0
    for i in range(start, len(text)):
        if text[i] == "[":
            depth += 1
        elif text[i] == "]":
            depth -= 1
            if depth == 0:
                return text[start + 1 : i]
    return None


def split_entries(body):
    """表 body 按顶层逗号切条目;跳过 lambda/字符串(字符串内逗号)。"""
    entries = []
    depth = 0
    cur = []
    i, n = 0, len(body)
    in_str = None
    while i < n:
        ch = body[i]
        if in_str:
            cur.append(ch)
            if ch == "\\":
                cur.append(body[i + 1] if i + 1 < n else "")
                i += 2
                continue
            if ch == in_str:
                in_str = None
            i += 1
            continue
        if ch in "\"'":
            in_str = ch
            cur.append(ch)
        elif ch in "([{":
            depth += 1
            cur.append(ch)
        elif ch in ")]}":
            depth -= 1
            cur.append(ch)
        elif ch == "," and depth == 0:
            entries.append("".join(cur).strip())
            cur = []
        else:
            cur.append(ch)
        i += 1
    if "".join(cur).strip():
        entries.append("".join(cur).strip())
    return [e for e in entries if e]


def match_paren(text, start):
    """从 text[start]=='(' 起配平,返回 (inner, end_idx_incl_closing)。"""
    depth = 0
    i = start
    in_str = None
    while i < len(text):
        ch = text[i]
        if in_str:
            if ch == "\\":
                i += 2
                continue
            if ch == in_str:
                in_str = None
            i += 1
            continue
        if ch in "\"'":
            in_str = ch
        elif ch == "(":
            depth += 1
        elif ch == ")":
            depth -= 1
            if depth == 0:
                return text[start + 1 : i], i
        i += 1
    raise ValueError(f"unbalanced parens at {start}: …{text[start:start+80]}…")


def split_args(inner):
    """参数串按顶层逗号切分。"""
    args = []
    depth = 0
    cur = []
    i, n = 0, len(inner)
    in_str = None
    while i < n:
        ch = inner[i]
        if in_str:
            if ch == "\\":
                cur.append(inner[i + 1] if i + 1 < n else "")
                i += 2
                continue
            if ch == in_str:
                in_str = None
            cur.append(ch)
            i += 1
            continue
        if ch in "\"'":
            in_str = ch
            cur.append(ch)
        elif ch in "([{":
            depth += 1
            cur.append(ch)
        elif ch in ")]}":
            depth -= 1
            cur.append(ch)
        elif ch == "," and depth == 0:
            args.append("".join(cur).strip())
            cur = []
        else:
            cur.append(ch)
        i += 1
    if "".join(cur).strip():
        args.append("".join(cur).strip())
    return args


# ---- finder 条目解析 ----

# 机械方法 → (参数个数限制, 说明);None = 不限制(剩余参数为 lambda 原文的
# 由调用侧检查)。值解析在 parse_value。
MECH_METHODS = {
    "UsingICD", "UsingOrigin", "UsingNotAccurate", "UsingTimeOffset",
    "UsingBeforeWeaponSwap", "UsingAfterWeaponSwap", "UsingEnable",
    "UsingDisableWithEffectData", "UsingDisableWithMissileData",
    "WithBuilds", "WithEvtcBuilds", "WithMinions",
    "UsingDurationChecker", "UsingByBaseSpecChecker", "UsingToBaseSpecChecker",
    "UsingBySpecChecker", "UsingByNotSpecChecker", "UsingBySpecsChecker",
    "UsingByNotSpecsChecker", "UsingToSpecChecker", "UsingToNotSpecChecker",
    "UsingToSpecsChecker", "UsingToNotSpecsChecker",
    "UsingSrcBaseSpecChecker", "UsingDstBaseSpecChecker",
    "UsingSrcNotBaseSpecChecker", "UsingDstNotBaseSpecChecker",
    "UsingSrcSpecChecker", "UsingSrcNotSpecChecker", "UsingSrcSpecsChecker",
    "UsingSrcNotSpecsChecker", "UsingDstSpecChecker", "UsingDstNotSpecChecker",
    "UsingDstSpecsChecker", "UsingDstNotSpecsChecker",
    "UsingIsAroundDstChecker", "UsingNotIsAroundDstChecker",
    "UsingSecondaryEffectSameSrcChecker", "UsingSecondaryEffectInvertedSrcChecker",
    "UsingSecondaryEffectSameSrcInvertedTypeChecker",
    "UsingSecondaryEffectInvertedSrcInvertedTypeChecker",
    "UsingSecondaryEffectSameSrcSameTypeChecker",
    "UsingSecondaryEffectInvertedSrcSameTypeChecker",
    "UsingNoSecondaryEffectSameSrcChecker",
    "UsingNoSecondaryEffectSameSrcCheckerOnSamePosition",
    "UsingNoSecondaryEffectInvertedSrcChecker",
    "UsingNoSecondaryEffectSameSrcInvertedTypeChecker",
    "UsingNoSecondaryEffectInvertedSrcInvertedTypeChecker",
    "UsingNoSecondaryEffectSameSrcSameTypeChecker",
    "UsingNoSecondaryEffectInvertedSrcSameTypeChecker",
    "UsingNoAnimatedCastChecker", "UsingAgentRedirectionIfUnknown",
}


def const_value(text, tables, ctx):
    """把 C# 常量/枚举引用解析为 JSON 值;失败返回 None。

    ctx: {"spec": 组名} 用于报错信息。支持:
      -123 / 0x2A → int
      SkillIDs 名 / SkillIDs 名(带 - 前缀?)
      EffectGUIDs.X → {"guid": hex}
      GW2Builds.X → int
      MinionID.X → int
      Spec.X / X → str spec 名
      InstantCastFinder.InstantCastOrigin.X → X 尾段
      WeaponSetIDs.KitSet 等 → 原样字符串?调用方按方法语义解释
    """
    t = text.strip()
    if not t:
        return None
    if t.startswith("(int)"):
        t = t[5:].strip()
    if re.fullmatch(r"-?\d+", t):
        return int(t)
    if re.fullmatch(r"0x[0-9a-fA-F]+", t):
        return int(t, 16)
    if t in tables["skills"]:
        return tables["skills"][t]
    if t.startswith("EffectGUIDs."):
        name = t[len("EffectGUIDs."):]
        return {"guid": tables["guids"].get(name)}
    if t.startswith("GW2Builds."):
        return tables["builds"].get(t[len("GW2Builds."):])
    if t.startswith("MinionID."):
        name = t[len("MinionID."):]
        return tables["minion"].get(name)
    if t.startswith("Spec."):
        return {"spec": t[len("Spec."):]}
    if "InstantCastOrigin." in t:
        # 形如 InstantCastFinder.InstantCastOrigin.Trait /
        # EIData.InstantCastFinder.InstantCastOrigin.Trait
        return t.rsplit(".", 1)[1]
    if t == "InstantCastFinder.DefaultICD":
        return 50
    # 局域集合(HashSet/数组)引用 —— 调用方先查局部集合并展开
    return None



# ---- lambda checker 结构抽取 ----
# 多数 UsingChecker 的 lambda 是「单查询调用」形态,这里抽取成结构化条件:
#   {"cond": <名>, "args": [值...], "neg": bool}
# 无法机械翻译的保留原文(引擎侧登记不支持,不静默)。

FETCHER_COND = {
    "HasGainedBuff": "has_gained_buff",
    "HasLostBuff": "has_lost_buff",
    "HasLostBuffStack": "has_lost_buff_stack",
    "HasRelatedEffectDst": "has_related_effect_dst",
    "HasRelatedEffect": "has_related_effect",
}


def _value_or_unresolved(t, tables):
    t = t.strip()
    v = const_value(t, tables, "lambda")
    if v is not None:
        return v
    if t.startswith("EffectGUIDs."):
        return {"guid": None}
    return {"unresolved": t}


def parse_fparam(t, tables):
    """fetcher 参数的模板值化:
       - 数字/常量 → int
       - evt.To / brae.By / effect.Src / evt.Dst → {"agent": to|by|src|dst}
       - evt.Time / effect.Time + 120 → {"t": 毫秒偏移}
       - EffectGUIDs.X → {"guid": hex}(缺失为 None)
    """
    t = t.strip()
    v = const_value(t, tables, "lambda")
    if v is not None:
        return v
    m = re.fullmatch(r"(?:\w+\.)?(Time)\s*(?:\+\s*(-?\d+))?", t)
    if m:
        return {"t": int(m.group(2) or 0)}
    m = re.fullmatch(r"(?:\w+\.)?(To|By|Src|Dst)", t)
    if m:
        return {"agent": m.group(1).lower()}
    if t.startswith("EffectGUIDs."):
        return {"guid": tables["guids"].get(t[len("EffectGUIDs."):])}
    return None


def translate_lambda(lam, tables):
    """lambda 源码 → {"cond",...} 或 {"custom": raw}。"""
    raw = lam.strip()
    s = raw
    # 仅剥「首尾括号恰好包住整个含 => 的表达式」的壳(如 Ranger 的
    # `((evt,...) => ...)`);lambda 形参括号不能剥
    while s.startswith("("):
        try:
            inner, end = match_paren(s, 0)
        except ValueError:
            break
        if end == len(s) - 1 and "=>" in inner:
            s = inner.strip()
            continue
        break
    arrow = s.find("=>")
    if arrow >= 0:
        s = s[arrow + 2 :].strip()
    if s.startswith("{") and s.endswith("}"):
        s = s[1:-1].strip()
        if s.startswith("return "):
            s = s[len("return "):]
        if s.endswith(";"):
            s = s[:-1]
    s = s.strip()
    neg = False
    if s.startswith("!"):
        neg = True
        s = s[1:].strip()
    # duration 近似
    m = re.fullmatch(r"Math\.Abs\((?:evt|ba)\.AppliedDuration\s*-\s*(-?\d+)\)\s*<\s*ServerDelayConstant", s)
    if m:
        return {"cond": "duration_approx", "args": [int(m.group(1))], "neg": False}
    m = re.fullmatch(r"Math\.Abs\((?:evt|ba)\.ExtendedDuration\s*-\s*(-?\d+)\)\s*<\s*ServerDelayConstant", s)
    if m:
        return {"cond": "extended_duration_approx", "args": [int(m.group(1))], "neg": False}
    # 复合:agent.IsSpecies(...) / evt.IsAroundDst && evt.Dst.IsSpecies(...)
    m = re.fullmatch(r"(?:(?:evt|effectEvent|brae|ba|blcf)\.)?(Src|Dst|To|By)\.IsSpecies\((MinionID\.\w+)\)", s)
    if m:
        v = tables["minion"].get(m.group(2)[len("MinionID."):])
        if v is not None:
            return {"cond": "is_species", "args": [m.group(1).lower(), v], "neg": neg}
    m = re.fullmatch(r"evt\.IsAroundDst\s*&&\s*evt\.Dst\.IsSpecies\((MinionID\.\w+)\)", s)
    if m:
        v = tables["minion"].get(m.group(1)[len("MinionID."):])
        if v is not None:
            return {"cond": "is_around_dst_species", "args": [v], "neg": neg}
    m = re.fullmatch(r"!?evt\.IsAroundDst", s)
    if m:
        return {"cond": "is_around_dst", "args": [], "neg": neg}
    # 单 fetcher 调用
    m = re.fullmatch(r"(?:combatData\.)?(\w+)\((.*)\)", s, re.S)
    if m and m.group(1) in FETCHER_COND:
        args = []
        ok = True
        for a in split_args(m.group(2)):
            v = parse_fparam(a, tables)
            if v is None or (isinstance(v, dict) and v.get("guid") is None and "unresolved" not in v and len(v) == 0):
                ok = False
                break
            args.append(v)
        if ok:
            return {"cond": FETCHER_COND[m.group(1)], "args": args, "neg": neg}

    # ---- 特判模式(人工核对的复杂 lambda;按文本特征定位) ----
    sid = tables.get("skills", {})
    # agentData.HasSpawnedMinion(MinionID.X, evt.Dst, evt.Time[, 30]) ——
    # 省略的 eps 取 C# 签名默认 ServerDelayConstant(AgentData.cs:225)。
    m = re.fullmatch(r"agentData\.HasSpawnedMinion\((MinionID\.\w+),\s*evt\.(Dst|Src),\s*evt\.Time(?:\s*,\s*(\d+))?\)", s)
    if m:
        sp = tables["minion"].get(m.group(1)[len("MinionID."):])
        if sp is not None:
            return {"cond": "has_spawned_minion", "args": [sp, m.group(2).lower(), 0, int(m.group(3) or SERVER_DELAY_CONST)], "neg": neg}
    # 单技能 damage-in-window 且 !另一技能(MindWrack / MindWrackAmmo 分流)
    m2 = re.fullmatch(
        r"combatData\.GetDamageData\((\w+)\)\.Any\(x => x\.CreditedFrom\.Is\(evt\.Src\) && Math\.Abs\(x\.Time - evt\.Time\) < (\d+)\) && !combatData\.GetDamageData\((\w+)\)\.Any\(x => x\.CreditedFrom\.Is\(evt\.Src\) && Math\.Abs\(x\.Time - evt\.Time\) < (\d+)\)",
        s,
    )
    if m2 and m2.group(1) in sid and m2.group(3) in sid:
        return {"cond": "two_skill_damage", "args": ["a_not_b", sid[m2.group(1)], sid[m2.group(3)], int(m2.group(2))], "neg": False}
    # MindWrackOrMindWrackAmmo 块:hasNormal/hasAmmo 两行
    m3 = re.search(r"var hasNormal = combatData\.GetDamageData\((\w+)\)\.Any\(x => x\.CreditedFrom\.Is\(evt\.Src\) && Math\.Abs\(x\.Time - evt\.Time\) < (\d+)\);", s)
    m4 = re.search(r"var hasAmmo = combatData\.GetDamageData\((\w+)\)\.Any\(x => x\.CreditedFrom\.Is\(evt\.Src\) && Math\.Abs\(x\.Time - evt\.Time\) < (\d+)\);", s)
    if m3 and m4 and m3.group(1) in sid and m4.group(1) in sid:
        return {"cond": "two_skill_damage", "args": ["either_both", sid[m3.group(1)], sid[m4.group(1)], int(m3.group(2))], "neg": False}
    # RemoveAll 窗口:FindRelatedEvents(GetBuffRemoveAllData(Buff), T[, window]).Any()
    # 注意 window 省略时取 C# 签名默认 ServerDelayConstant=10(CombatDataHelpers.
    # cs:8),不能给 0 —— 0 会让引擎窗口 |t-ev| < 0 恒不命中(NecromancerHelper
    # 的 SpectralRecall 10687 / Distress 73116 调用省略窗口)。
    m5 = re.fullmatch(
        r"(?:CombatData\.)?FindRelatedEvents\(combatData\.GetBuffRemoveAllData\((\w+)\),\s*(?:evt|effectEvent)\.Time(?:\s*\+\s*(\d+))?(?:,\s*(\d+))?\)\.Any\(\)",
        s,
    )
    if m5 and m5.group(1) in sid:
        return {"cond": "recent_remove_all", "args": [sid[m5.group(1)], int(m5.group(2) or 0), int(m5.group(3) or SERVER_DELAY_CONST)], "neg": neg}
    # MineDetonationInstantCastChecker(effect, combatData, ifFound, [GUIDs])
    m6 = re.fullmatch(r"MineDetonationInstantCastChecker\(effect,\s*combatData,\s*(true|false),\s*\[\s*(.*?)\s*\]\)", s, re.S)
    if m6:
        parts = split_args(m6.group(2))
        guids = []
        for part in parts:
            g = tables["guids"].get(part.replace("EffectGUIDs.", "").strip())
            if g is None:
                guids = None
                break
            guids.append(g)
        if guids is not None:
            return {"cond": "mine_detonation", "args": [m6.group(1) == "true", guids], "neg": False}
    # Guardian buff-apply 窗口(Advance: aegis 20000-40000;StandYourGround: stability >= 5)
    m7 = re.search(r"GetBuffApplyDataByIDBySrc\((\w+),\s*evt\.Dst\)", s)
    if m7 and m7.group(1) in sid:
        if "AppliedDuration + ServerDelayConstant >= 20000" in s:
            return {"cond": "buff_apply_window", "args": [sid[m7.group(1)], "dst", 20000, 40000, 1], "neg": False}
        if "5 <= CombatData.FindRelatedEvents" in s:
            return {"cond": "buff_apply_window", "args": [sid[m7.group(1)], "dst", 0, 0, 5], "neg": False}
    # Sadistic Searing(BuffLoss):10s - removed 的 damage 窗口
    m8 = re.search(r"long sadisticSearingDuration = 10000 - blcf\.RemovedDuration;", s)
    if m8:
        m8b = re.search(r"GetDamageData\((\w+)\)\.Any", s)
        if m8b and m8b.group(1) in sid:
            return {"cond": "sadistic_damage_window", "args": [sid[m8b.group(1)]], "neg": False}
    mh = re.fullmatch(r"combatData\.HasRelatedHit\((\w+),\s*evt\.(Src|Dst|To|By),\s*evt\.Time\)", s)
    if mh and mh.group(1) in sid:
        return {"cond": "has_related_hit", "args": [sid[mh.group(1)], mh.group(2).lower(), 0], "neg": neg}
    # !combatData.IsCasting(Skill, agent, time)(Elementalist GrandFinale 系)
    mc = re.fullmatch(r"combatData\.IsCasting\((\w+),\s*ba\.(To|By),\s*ba\.Time\)", s)
    if mc and mc.group(1) in sid:
        return {"cond": "is_casting", "args": [sid[mc.group(1)], mc.group(2).lower(), 0], "neg": neg}
    # Relic of the Claw:同 instance 的 RemoveSingle 距今 < 10ms 则不命中
    m9 = re.search(r"GetBuffRemoveSingleDataByIDByDst\((\w+),\s*ba\.To\)", s)
    if m9 and m9.group(1) in sid:
        return {"cond": "relic_claw_no_recent_single", "args": [sid[m9.group(1)]], "neg": False}
    return {"custom": raw}

def parse_finder_entry(entry, tables):
    """单条目 → dict;失败抛 ParseError 信息。"""
    text = entry
    # 找到 `new <Class>(` 起点
    # 类名 = <Kind>CastFinder 或 EngineerKitFinder(不带动词 Cast ——
    # EngineerHelper.cs:16 的有状态子类,引擎手工移植)
    m = re.search(r"\bnew\s+((?:\w+CastFinder)|(?:EngineerKitFinder))\w*\s*\(", text)
    if not m:
        return {"error": "no ctor", "raw": text}
    cls = m.group(1)
    if cls == "EngineerKitFinder":
        # 有状态子类(EngineerHelper.cs:16-84):per-caster 双游标 —— 不走
        # 表,引擎手工移植;这里只登记产出技能。
        ctor_args_raw2, _ = match_paren(text, m.end() - 1)
        return {
            "kind": "EngineerKit",
            "args": split_args(ctor_args_raw2),
            "methods": [],
            "custom_checkers": [],
            "raw": text[:120],
        }
    if cls not in FINDER_CLASSES:
        return {"error": f"ctor class {cls}", "raw": text}
    ctor_args_raw, end = match_paren(text, m.end() - 1)
    rest = text[end + 1 :].strip()
    out = {
        "kind": cls,
        "args": [],
        "methods": [],
        "custom_checkers": [],
        "raw": text[:300],
    }
    # 构造参数:整段文本解析(常量/集合引用)。
    for a in split_args(ctor_args_raw):
        out["args"].append(a)  # 先原文,下面语义化由 kind 决定
    # fluent 链:.Method(...) 或 .UsingXxx(...)
    i = 0
    n = len(rest)
    while i < n:
        while i < n and (rest[i].isspace() or rest[i] == "."):
            i += 1
        if i >= n:
            break
        j = i
        while j < n and (rest[j].isalnum() or rest[j] == "_"):
            j += 1
        name = rest[i:j]
        i = j
        while i < n and rest[i].isspace():
            i += 1
        if i < n and rest[i] == "(":
            inner, end = match_paren(rest, i)
            i = end + 1
        else:
            inner = ""
        out["methods"].append({"name": name, "args": split_args(inner)})
    # 检查 args 常量解析(按 kind 语义:第二参可能是 buff/damage id/guid/
    # species/minion 枚举等 —— 全部统一先查常量表)
    return out


def expand_local_set(ref, file_text, tables, fname):
    """展开 `(int)MinionID.X` 集合引用(如 RangerHelper 的 JuvenilePetIDs)。
    返回成员数值列表或 None(解析失败)。"""
    m = re.search(
        r"(?:private\s+static\s+readonly|static\s+readonly)\s+"
        r"HashSet<int>\s+" + re.escape(ref) + r"\s*=\s*new\s+HashSet<int>\s*\{(.*?)\};",
        file_text,
        re.S,
    )
    if not m:
        # 也允许 int[] 集合初始化器
        m = re.search(
            r"readonly\s+(?:HashSet<int>|int\[\])\s+" + re.escape(ref) + r"\s*=\s*(?:new\s+(?:HashSet<int>|int\[\])\s*)?\{(.*?)\};",
            file_text,
            re.S,
        )
    if not m:
        return None
    vals = []
    for part in split_entries(m.group(1)):
        v = const_value(part, tables, fname)
        if v is None and part.startswith("(int)"):
            v = const_value(part, tables, fname)
        if isinstance(v, int):
            vals.append(v)
        else:
            return None
    return vals


def resolve_arg_text(a, tables, file_local_sets, fname):
    """参数原文 → JSON 值;优先集合展开。"""
    v = const_value(a, tables, fname)
    if v is not None:
        return v
    # 局域集合名?
    if a in file_local_sets:
        return {"idset": file_local_sets[a]}
    if a.startswith("EffectGUIDs."):
        return {"guid": None}  # unresolved guid 常量
    return {"unresolved": a}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--src", default=str(DEFAULT_SRC))
    ap.add_argument("--out", default=str(REPO / "content" / "instant-cast-finders.json"))
    args = ap.parse_args()
    src = Path(args.src)
    ei = src / "GW2EI.Library" / "GW2EI.Services" / "GW2EIEvtcParser"
    prof = ei / "EIData" / "ProfHelpers"
    if not prof.exists():
        print(f"error: ProfHelpers not found under {src}", file=sys.stderr)
        sys.exit(2)
    ids_dir = ei / "ParserHelpers" / "IDs"

    tables = {
        "skills": parse_skill_ids(strip_comments((ids_dir / "SkillIDs.cs").read_text(encoding="utf-8"))),
        "guids": parse_guid_consts(strip_comments((ei / "ParserHelpers" / "GUIDs" / "EffectGUIDs.cs").read_text(encoding="utf-8"))),
        "builds": parse_gw2_builds(strip_comments((ei / "ParserHelpers" / "GW2Builds.cs").read_text(encoding="utf-8"))),
        "minion": parse_minion_id_enum(strip_comments((ids_dir / "SpeciesIDs.cs").read_text(encoding="utf-8"))),
    }
    print(f"consts: skills={len(tables['skills'])} guids={len(tables['guids'])} "
          f"builds={len(tables['builds'])} minion={len(tables['minion'])}")

    # ---- ProfHelper.cs 通用表 ----
    ph_text = strip_comments((prof / "ProfHelper.cs").read_text(encoding="utf-8"))
    # 局部集合:通用表没有集合引用
    profhelper = []
    for field in ("_genericNeedsToBeBeforeTheRestInstantCastFinders_NeverAddAnythingElse",
                  "_genericInstantCastFinders"):
        body = find_table(ph_text, field)
        if body is None:
            print(f"error: table {field} not found in ProfHelper.cs", file=sys.stderr)
            sys.exit(2)
        for e in split_entries(body):
            if e.startswith("#"):
                continue
            parsed = parse_finder_entry(e, tables)
            parsed["table"] = "generic"
            profhelper.append(parsed)

    # ---- Helper 文件表 ----
    helpers = {}
    unresolved_all = []
    for group, files in HELPERS.items():
        for spec_name, rel in files.items():
            f = prof / rel
            text = strip_comments(f.read_text(encoding="utf-8"))
            body = find_table(text, "InstantCastFinder")
            if body is None:
                print(f"error: no InstantCastFinder table in {rel}", file=sys.stderr)
                sys.exit(2)
            # 文件内局域集合(如 JuvenilePetIDs;含 Union 链)
            local_sets = collect_local_sets(text, tables)
            entries = []
            for e in split_entries(body):
                if e.startswith("#"):
                    continue  # #region/#endregion 残留
                parsed = parse_finder_entry(e, tables)
                if parsed.get("error"):
                    unresolved_all.append({**parsed, "file": rel, "spec": spec_name})
                    continue
                # 构造参数解析(统一尝试;skill/第二参语义按 kind 由引擎处理)
                resolved_args = []
                for a in parsed["args"]:
                    v = resolve_arg_text(a, tables, local_sets, rel)
                    resolved_args.append(v)
                parsed["args"] = resolved_args
                # 机械 fluent 方法白名单检查;UsingChecker 的 lambda 做结构
                # 抽取(可模板化 → methods;否则进 custom 登记)
                mech, custom = [], []
                for meth in parsed["methods"]:
                    name = meth["name"]
                    if name in MECH_METHODS and name != "UsingChecker":
                        margs = []
                        for a in meth["args"]:
                            v = resolve_arg_text(a, tables, local_sets, rel)
                            margs.append(v)
                        mech.append({"name": name, "args": margs})
                    elif name == "UsingChecker" and meth["args"]:
                        tr = translate_lambda(meth["args"][0], tables)
                        if "cond" in tr:
                            mech.append({"name": "LambdaCond", "cond": tr})
                        else:
                            custom.append(meth)
                    else:
                        custom.append(meth)
                parsed["methods"] = mech
                parsed["custom_checkers"] = custom
                if custom:
                    unresolved_all.append({
                        "file": rel, "spec": spec_name, "kind": parsed["kind"],
                        "custom": custom, "raw": parsed["raw"],
                    })
                entries.append(parsed)
            helpers.setdefault(group, {})[spec_name] = entries
            print(f"helper {group}/{spec_name}: {len(entries)} finders "
                  f"({len(entries) - len([e for e in entries if e.get('error')])} ok)")

    # ProfHelper 通用表条目也要做 args/methods 处理
    done_prof = []
    for parsed in profhelper:
        if parsed.get("error"):
            unresolved_all.append({**parsed, "file": "ProfHelper.cs", "spec": "generic"})
            continue
        resolved_args = []
        for a in parsed["args"]:
            resolved_args.append(resolve_arg_text(a, tables, {}, "ProfHelper.cs"))
        parsed["args"] = resolved_args
        mech, custom = [], []
        for meth in parsed["methods"]:
            if meth["name"] in MECH_METHODS and meth["name"] != "UsingChecker":
                margs = [resolve_arg_text(a, tables, {}, "ProfHelper.cs") for a in meth["args"]]
                mech.append({"name": meth["name"], "args": margs})
            elif meth["name"] == "UsingChecker" and meth["args"]:
                tr = translate_lambda(meth["args"][0], tables)
                if "cond" in tr:
                    mech.append({"name": "LambdaCond", "cond": tr})
                else:
                    custom.append(meth)
            else:
                custom.append(meth)
        parsed["methods"] = mech
        parsed["custom_checkers"] = custom
        if custom:
            unresolved_all.append({
                "file": "ProfHelper.cs", "spec": "generic", "kind": parsed["kind"],
                "custom": custom, "raw": parsed["raw"],
            })
        done_prof.append(parsed)

    doc = {
        "generated_from": "3b7278f9b",
        "note": "P3b 最小子集:ProfHelper 通用表 + 15 Helper 文件(extract-instant-casts.py)",
        "generic": done_prof,
        "helpers": helpers,
        "unresolved": unresolved_all,
    }
    out_path = Path(args.out)
    out_path.write_text(json.dumps(doc, ensure_ascii=False, indent=1), encoding="utf-8")
    total = len(done_prof) + sum(len(v) for g in helpers.values() for v in g.values())
    print(f"total finders: {total} (generic {len(done_prof)})")
    print(f"unresolved/custom entries: {len(unresolved_all)}")
    for u in unresolved_all[:30]:
        print("  UNRES:", u.get("file"), u.get("spec"), u.get("kind"), u.get("raw", "")[:90])
    print(f"wrote {out_path}")


if __name__ == "__main__":
    main()
