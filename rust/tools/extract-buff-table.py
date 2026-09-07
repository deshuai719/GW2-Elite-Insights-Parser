#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Extract EI Buff registry and SkillItem override tables from C# sources into
rust/content/*.json (data-internalization pipeline, P2b).

Only reads C# sources under GW2EI.Library/GW2EI.Services/GW2EIEvtcParser/; never
modifies them. The golden JSON is the contract; when the C# source changes
upstream, re-run this script and commit the regenerated JSON.

Usage:
    python rust/tools/extract-buff-table.py [path-to-GW2EIEvtcParser]
"""
import json
import os
import re
import sys
from pathlib import Path

GW2BUILDS_END = 18446744073709551615  # ulong.MaxValue
GW2BUILDS_START = 0                   # ulong.MinValue
ARCDPS_END = 2147483647               # int.MaxValue
ARCDPS_START = -2147483648            # int.MinValue

# ---------------------------------------------------------------- helpers

_STR_ESC = {
    '\\': '\\', '"': '"', "'": "'", 'n': '\n', 't': '\t', 'r': '\r',
    '0': '\0', 'a': '\a', 'b': '\b', 'f': '\f', 'v': '\v',
}


def parse_cs_string(s: str) -> str:
    out = []
    i = 0
    while i < len(s):
        c = s[i]
        if c == '\\' and i + 1 < len(s):
            nxt = s[i + 1]
            if nxt == 'u' and i + 6 <= len(s) and all(ch in '0123456789abcdefABCDEF' for ch in s[i + 2:i + 6]):
                out.append(chr(int(s[i + 2:i + 6], 16)))
                i += 6
                continue
            out.append(_STR_ESC.get(nxt, nxt))
            i += 2
        else:
            out.append(c)
            i += 1
    return ''.join(out)


def strip_comments(src: str) -> str:
    out = []
    i = 0
    n = len(src)
    while i < n:
        c = src[i]
        nxt = src[i + 1] if i + 1 < n else ''
        if c == '/' and nxt == '/':
            while i < n and src[i] != '\n':
                i += 1
            continue
        if c == '/' and nxt == '*':
            i += 2
            while i + 1 < n and not (src[i] == '*' and src[i + 1] == '/'):
                i += 1
            i += 2
            continue
        if c == '"':
            j = i + 1
            while j < n:
                if src[j] == '\\':
                    j += 2
                    continue
                if src[j] == '"':
                    break
                j += 1
            out.append(src[i:j + 1])
            i = j + 1
            continue
        out.append(c)
        i += 1
    return ''.join(out)


def split_top(s: str, sep=','):
    parts = []
    depth = 0
    cur = []
    i = 0
    while i < len(s):
        c = s[i]
        if c == '"':
            j = i + 1
            while j < len(s):
                if s[j] == '\\':
                    j += 2
                    continue
                if s[j] == '"':
                    break
                j += 1
            cur.append(s[i:j + 1])
            i = j + 1
            continue
        if c in '([{':
            depth += 1
        elif c in ')]}':
            depth -= 1
        if c == sep and depth == 0:
            parts.append(''.join(cur).strip())
            cur = []
        else:
            cur.append(c)
        i += 1
    if ''.join(cur).strip():
        parts.append(''.join(cur).strip())
    return parts


# ---------------------------------------------------------------- constants

CONST_RE = re.compile(
    r'(?:public|internal|private|static|readonly|\s)*'
    r'(?:const|static readonly)\s+(ulong|long|int|uint|ushort|string)\s+(\w+)\s*=\s*(.+?);'
)
# name -> (is_string, value)
CONSTANTS = {}


def load_constants(root: Path):
    for p in root.rglob('*.cs'):
        txt = strip_comments(p.read_text(encoding='utf-8-sig', errors='replace'))
        for m in CONST_RE.finditer(txt):
            ty, name, val = m.group(1), m.group(2), m.group(3).strip()
            key = f'{p.name}::{name}'
            if ty == 'string':
                if val.startswith('"') and val.endswith('"'):
                    CONSTANTS[key] = ('str', parse_cs_string(val[1:-1]))
                continue
            neg = ''
            body = val
            if body.startswith('-'):
                neg = '-'
                body = body[1:]
            special = {'ulong.MinValue': GW2BUILDS_START, 'ulong.MaxValue': GW2BUILDS_END,
                       'int.MinValue': ARCDPS_START, 'int.MaxValue': ARCDPS_END,
                       'long.MinValue': -(1 << 63), 'long.MaxValue': (1 << 63) - 1}
            if body in special:
                CONSTANTS[key] = ('int', special[body])
                continue
            body = body.replace('_', '')
            if body.isdigit():
                CONSTANTS[key] = ('int', int(neg + body))
    bare = {}
    for k, v in CONSTANTS.items():
        bare.setdefault(k.split('::')[-1], v)
    # string constants can collide with int constants of the same name
    # (SkillIDs.Confusion vs BuffImages.Confusion): keep separate indexes.
    bare_int = {}
    bare_str = {}
    for k, v in CONSTANTS.items():
        b = k.split('::')[-1]
        if v[0] == 'str':
            bare_str.setdefault(b, v)
        else:
            bare_int.setdefault(b, v)
    return CONSTANTS, bare, bare_int, bare_str


SPECIAL = {'ulong.MinValue': GW2BUILDS_START, 'ulong.MaxValue': GW2BUILDS_END,
           'int.MinValue': ARCDPS_START, 'int.MaxValue': ARCDPS_END,
           'long.MinValue': -(1 << 63), 'long.MaxValue': (1 << 63) - 1}


def resolve_bare(bare_consts, name):
    if name in SPECIAL:
        return ('int', SPECIAL[name])
    if name in bare_consts:
        return bare_consts[name]
    return None


INT_RE = re.compile(r'^[-+]?[0-9][0-9_]*$')


def classify_value(tok, bare_consts):
    """-> ('str', s) | ('int', n) | ('enum', last) | ('sym', name) | None"""
    tok = tok.strip()
    if not tok:
        return None
    if tok.startswith('"'):
        if not tok.endswith('"'):
            return None
        return ('str', parse_cs_string(tok[1:-1]))
    if INT_RE.match(tok):
        return ('int', int(tok.replace('_', '')))
    m = re.match(r'^([A-Za-z_]\w*)((?:\.[A-Za-z_]\w*)*)$', tok)
    if m:
        segs = tok.split('.')
        if len(segs) > 1:
            head = '.'.join(segs[:-1])
            if head in ('ulong', 'int', 'long'):
                return ('int', SPECIAL.get(tok, None))
            if head in ('BuffImages', 'ItemImages', 'ParserIcons', 'TraitImages', 'SkillImages'):
                return ('iconref', tok)
            return ('enum', segs[-1])
        r = resolve_bare(bare_consts, tok)
        if r:
            return r
        return ('sym', tok)
    return None


# ---------------------------------------------------------------- buff extraction

CLASSIF_ENUM = {
    'Condition', 'Boon', 'Offensive', 'Defensive', 'Support', 'Debuff',
    'Gear', 'Other', 'Enhancement', 'Nourishment', 'OtherConsumable',
    'Hidden', 'Unknown',
}
STACKTYPE_ENUM = {
    'Queue', 'Regeneration', 'Force', 'Stacking',
    'StackingUniquePerSrc', 'StackingConditionalLoss',
}
SOURCE_ENUM = {
    'Common', 'FractalInstability', 'Item', 'ItemUnknown', 'Nature',
    'Extension', 'Gadget', 'PlayerCondition', 'PlayerBoon', 'EnemyBoon',
    'Skill', 'Profession', 'Guild', 'WvW', 'Combo', 'Unknown', 'Unkown',
    'FightSpecific',
}

BUFF_LIST_FILES = [
    'EIData/Buffs/CommonBuffs.cs',
    'EIData/Buffs/FoodBuffs.cs',
    'EIData/Buffs/UtilityBuffs.cs',
    'EIData/Buffs/EncounterBuffs.cs',
    'EIData/Buffs/WvWBuffs.cs',
]


def parse_call_and_tail(txt, start):
    """Parse `new Buff(` at start: returns (args, end_pos) or None.
    end_pos points past the closing paren (tail `.WithBuilds(...)` follows)."""
    i = txt.find('(', start)
    if i < 0:
        return None
    depth = 0
    j = i
    n = len(txt)
    while j < n:
        c = txt[j]
        if c == '"':
            jj = j + 1
            while jj < n:
                if txt[jj] == '\\':
                    jj += 2
                    continue
                if txt[jj] == '"':
                    break
                jj += 1
            j = jj
        elif c == '(':
            depth += 1
        elif c == ')':
            depth -= 1
            if depth == 0:
                return split_top(txt[i + 1:j]), j + 1
        j += 1
    return None


def collect_buff_calls(root: Path):
    """(listvar, abs_pos_in_raw_file, Path, raw_txt) tuples for every buff list."""
    calls = []

    def scan(fp: Path, txt: str):
        member_re = re.compile(
            r'(?:public|internal|private|static|readonly|\s)*'
            r'(?:IReadOnlyList<Buff>|List<Buff>)\s+(\w+)\s*=(?!=)')
        seen = set()
        for m in member_re.finditer(txt):
            name = m.group(1)
            b = txt.find('[', m.end())
            if b < 0:
                continue
            end = txt.find(']', b)
            if end < 0:
                continue
            for mm in re.finditer(r'new Buff\(', txt[b:end]):
                pos = b + mm.start()
                seen.add(pos)
                calls.append((name, pos, fp, txt))
        # whole-file scan catches lists not matching the member regex (e.g.
        # built via helper functions); every call is recorded but nested/dup
        # handled by caller de-dup.
        for mm in re.finditer(r'new Buff\(', txt):
            pos = mm.start()
            if pos in seen:
                continue
            # only accept calls not inside another new Buff(...) (look backwards
            # at depth is expensive; instead filter: any enclosing Buff list in
            # the same member body is fine - plain adds.)
            calls.append(('(file)', pos, fp, txt))

    for rel in BUFF_LIST_FILES:
        fp = root / rel
        if fp.exists():
            scan(fp, fp.read_text(encoding='utf-8-sig', errors='replace'))
    # C# BuffsContainer `AllBuffs` order (BuffsContainer.cs:23-94), which decides
    # which definition wins when two Buffs share an id (GroupBy first).
    helper_order = [
        'RevenantHelper', 'HeraldHelper', 'RenegadeHelper', 'VindicatorHelper', 'ConduitHelper',
        'WarriorHelper', 'BerserkerHelper', 'SpellbreakerHelper', 'BladeswornHelper', 'ParagonHelper',
        'GuardianHelper', 'DragonhunterHelper', 'FirebrandHelper', 'WillbenderHelper', 'LuminaryHelper',
        'RangerHelper', 'DruidHelper', 'SoulbeastHelper', 'UntamedHelper', 'GaleshotHelper',
        'ThiefHelper', 'DaredevilHelper', 'DeadeyeHelper', 'SpecterHelper', 'AntiquaryHelper',
        'EngineerHelper', 'ScrapperHelper', 'HolosmithHelper', 'MechanistHelper', 'AmalgamHelper',
        'MesmerHelper', 'ChronomancerHelper', 'MirageHelper', 'VirtuosoHelper', 'TroubadourHelper',
        'NecromancerHelper', 'ReaperHelper', 'ScourgeHelper', 'HarbingerHelper', 'RitualistHelper',
        'ElementalistHelper', 'TempestHelper', 'WeaverHelper', 'CatalystHelper', 'EvokerHelper',
    ]
    by_dir = {}
    for hp in (root / 'EIData' / 'ProfHelpers').rglob('*Helper.cs'):
        by_dir[hp.stem] = hp
    for hname in helper_order:
        hp = by_dir.get(hname)
        if hp is None:
            print(f'  [missing helper] {hname}', file=sys.stderr)
            continue
        scan(hp, hp.read_text(encoding='utf-8-sig', errors='replace'))
    return calls


def parse_builds_tail(txt, pos, bare_consts, bare_int):
    """Parse consecutive .WithBuilds(...)/.WithEvtcBuilds(...) after pos."""
    min_gw2, max_gw2 = GW2BUILDS_START, GW2BUILDS_END
    min_evtc, max_evtc = ARCDPS_START, ARCDPS_END
    cur = pos
    while True:
        m = re.match(r'\s*\.(WithBuilds|WithEvtcBuilds)\(', txt[cur:])
        if not m:
            break
        call = parse_call_and_tail(txt, cur + txt[cur:].find('('))
        if call is None:
            break
        args, end = call
        vals = []
        ok = True
        for a in args:
            kl = classify_value(a, bare_consts)
            if kl is None:
                ok = False
                break
            if kl[0] in ('int',):
                vals.append(kl[1])
            elif kl[0] in ('sym', 'enum'):
                # GW2Builds.X / ArcDPSBuilds.X const references
                r = resolve_bare(bare_int, kl[1])
                if r and r[0] == 'int':
                    vals.append(r[1])
                else:
                    ok = False
                    break
            else:
                ok = False
                break
        if not ok or not vals:
            break
        if m.group(1) == 'WithBuilds':
            min_gw2 = vals[0]
            max_gw2 = vals[1] if len(vals) > 1 else GW2BUILDS_END
        else:
            min_evtc = vals[0]
            max_evtc = vals[1] if len(vals) > 1 else ARCDPS_END
        cur = end
    return min_gw2, max_gw2, min_evtc, max_evtc


def resolve_buff(args, tail, bare_consts, bare_str):
    kinds = [classify_value(a, bare_consts) for a in args]
    # name: first str, else first sym/enum token that does not look like an id
    name = None
    idv = None
    capacity = 1
    icon = ''
    classification = 'Unknown'
    stack_type = 'Unknown'
    sources = []
    seen_int = 0
    for k in kinds:
        if k is None:
            continue
        kind, val = k
        if kind == 'int':
            if idv is None:
                idv = val
            elif capacity == 1 and seen_int:
                capacity = val
            seen_int += 1
        elif kind == 'str':
            if name is None:
                name = val
            elif not icon:
                icon = val
        elif kind == 'enum':
            if val in CLASSIF_ENUM:
                classification = val
            elif val in STACKTYPE_ENUM:
                stack_type = val
            elif val in SOURCE_ENUM:
                sources.append(val)
            elif val in ('Duration', 'Intensity', 'Unknown'):
                pass
            else:
                # icon constant (BuffImages.X / ItemImages.X / ParserIcons.X / SkillImages.X)
                r = resolve_bare(bare_str, val)
                if r and r[0] == 'str' and not icon:
                    icon = r[1]
        elif kind == 'iconref':
            cls, _, mem = val.partition('.')
            r = CONSTANTS.get(f'{cls}.cs::{mem}')
            if r and r[0] == 'str' and not icon:
                icon = r[1]
        elif kind == 'sym':
            r = resolve_bare(bare_consts, val)
            if r and r[0] == 'int':
                if idv is None:
                    idv = r[1]
                else:
                    capacity = r[1]
            elif r and r[0] == 'str' and not icon:
                icon = r[1]
            elif name is None:
                name = val
    # reorder capacity detection: ints other than first int are capacities
    ints = [k[1] for k in kinds if k and k[0] == 'int']
    if len(ints) > 1:
        idv = ints[0]
        capacity = ints[1]
    elif len(ints) == 1:
        idv = ints[0]
    min_gw2, max_gw2, min_evtc, max_evtc = tail
    # last resort: bare symbol that resolves to a string constant (icon) - covers
    # icon names shadowed by stack-type enum members (e.g. Regeneration)
    if not icon:
        for k in kinds:
            if k and k[0] in ('enum', 'sym'):
                r = resolve_bare(bare_str, k[1])
                if r and r[0] == 'str':
                    icon = r[1]
                    break
    # C# 短构造器（Buff.cs:120-125：`Buff(name,id,Source(s),classification,link)`
    # 委托到 BuffStackType.Force, capacity 1）—— 参数里没有 stack 枚举 token
    # 但有 classification 时 = 短构造器 → Force/1。仅 3 参公共构造器
    # （Buff.cs:141-146：Name/ID/link，classification Unknown）保持 Unknown。
    if stack_type == 'Unknown' and classification != 'Unknown':
        stack_type = 'Force'
    if idv is None or name is None:
        raise ValueError(f'no id/name: args={args}')
    return {
        'id': idv, 'name': name, 'classification': classification,
        'stack_type': stack_type, 'capacity': capacity, 'icon': icon,
        'sources': sources,
        'min_gw2_build': min_gw2, 'max_gw2_build': max_gw2,
        'min_evtc_build': min_evtc, 'max_evtc_build': max_evtc,
    }


def main():
    root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(
        r'GW2EI.Library/GW2EI.Services/GW2EIEvtcParser')
    print(f'extracting buff table from {root}')
    load_constants(root)
    print(f'  constants: {len(CONSTANTS)} symbols')
    _, bare_consts, bare_int, bare_str = load_constants(root)

    calls = collect_buff_calls(root)
    print(f'  new Buff( occurrences: {len(calls)}')
    seen_pos = set()
    out = []
    problems = []
    for listvar, pos, fp, txt in calls:
        if (fp, pos) in seen_pos:
            continue
        seen_pos.add((fp, pos))
        parsed = parse_call_and_tail(txt, pos)
        if parsed is None:
            problems.append((fp.name, listvar, 'cannot parse call'))
            continue
        args, endpos = parsed
        tail = parse_builds_tail(txt, endpos, bare_consts, bare_int)
        try:
            b = resolve_buff(args, tail, bare_consts, bare_str)
            b['file'] = fp.name
            b['list'] = listvar
            out.append(b)
        except ValueError as e:
            problems.append((fp.name, listvar, str(e)))
    for f, l, msg in problems:
        print(f'  [unresolved] {f} {l}: {msg}', file=sys.stderr)
    # keep source collection order (C# `AllBuffs` list order) - it decides
    # which definition wins when two Buffs share one id (BuffsByIDs first).
    dst = Path(__file__).resolve().parent.parent / 'content' / 'buff-table.json'
    dst.write_text(json.dumps({'buff': out}, ensure_ascii=False, indent=1), encoding='utf-8')
    print(f'  wrote {dst}: {len(out)} buff definitions ({len(problems)} unresolved)')
    byc = {}
    for b in out:
        byc[b['classification']] = byc.get(b['classification'], 0) + 1
    print('  by classification:', dict(sorted(byc.items())))
    # sanity spot checks
    for (nm, bid) in [('Might', 740), ('Burning', 737), ('Confusion', 861),
                      ('Downed', 943), ('Sand Shade', -51)]:
        hit = [b for b in out if b['id'] == bid]
        if hit:
            print(f'  spot {nm}:', {k: hit[0][k] for k in ('id', 'name', 'classification', 'stack_type', 'capacity', 'min_gw2_build')})


if __name__ == '__main__':
    main()
