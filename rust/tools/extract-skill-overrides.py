#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Extract SkillItemOverrides (names/icons/NonCritableSkills) from the C#
SkillItemOverrides.cs into rust/content/skill-overrides.json.

Usage: python rust/tools/extract-skill-overrides.py [path-to-EvtcParser-dir]
"""
import json
import re
import sys
from pathlib import Path

# reuses the constant loader from extract-buff-table.py (importlib: filename has '-')
import importlib.util as _ilu
_buff_tool = Path(__file__).resolve().parent / 'extract-buff-table.py'
_spec = _ilu.spec_from_file_location('extract_buff_table', _buff_tool)
extract_buff_table = _ilu.module_from_spec(_spec)
_spec.loader.exec_module(extract_buff_table)
load_constants = extract_buff_table.load_constants

SPECIAL = {'ulong.MinValue': 0, 'ulong.MaxValue': 18446744073709551615,
           'int.MinValue': -2147483648, 'int.MaxValue': 2147483647}


def main():
    root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(
        r'GW2EI.Library/GW2EI.Services/GW2EIEvtcParser')
    fp = root / 'ParsedData' / 'Skills' / 'SkillItemOverrides.cs'
    txt = fp.read_text(encoding='utf-8-sig', errors='replace')
    # section split by dictionary declarations
    names_s = txt.split('OverridenSkillNames = new()', 1)[1]
    icons_s = txt.split('OverridenSkillIcons = new()', 1)[1]
    noncrit_s = txt.split('NonCritableSkills = new()', 1)[1]

    _, _, bare_int, bare_str = load_constants(root)

    def resolve_int(name):
        if name in SPECIAL:
            return SPECIAL[name]
        hit = bare_int.get(name)
        return hit[1] if hit else None

    def resolve_str(name):
        hit = bare_str.get(name)
        return hit[1] if hit else None

    def parse_pairs(body, val_kind):
        out = {}
        # stop at the closing brace of the dictionary initializer
        end = body.index('\n    };')
        body = body[:end]
        for m in re.finditer(r'\{\s*([A-Za-z_]\w*)\s*,\s*("(?:[^"\\]|\\.)*"|[A-Za-z_]\w*(?:\.[A-Za-z_]\w*)*|-?\d+)\s*\}', body):
            key_sym, val = m.group(1), m.group(2)
            kid = resolve_int(key_sym)
            if kid is None:
                print(f'  [skip] unresolved key {key_sym} ({fp.name})', file=sys.stderr)
                continue
            if val.startswith('"'):
                v = bytes(val[1:-1], 'utf-8').decode('unicode_escape')
                # unicode_escape mis-handles \uXXXX? keep simple; names have no escapes
            elif re.match(r'^-?\d+$', val):
                v = int(val)
            else:
                # 值符号可带命名空间(GW2Builds.StartOfLife 等)。裸名索引有
                # 跨文件同名冲突(ArcDPSBuilds.cs 与 GW2Builds.cs 都有
                # StartOfLife,值不同)—— 优先按 `<文件>.cs::<符号>` 全名匹配。
                v = None
                if '.' in val:
                    ns, _, sym = val.rpartition('.')
                    if (ns + '.cs::' + sym) in extract_buff_table.CONSTANTS:
                        v = extract_buff_table.CONSTANTS[ns + '.cs::' + sym][1]
                if v is None:
                    seg = val.split('.')[-1]
                    v = resolve_int(seg) if val_kind == 'int' else resolve_str(seg)
                if v is None:
                    print(f'  [skip] unresolved value {val} for key {key_sym}', file=sys.stderr)
                    continue
            out[str(kid)] = v
        return out

    names = parse_pairs(names_s, 'str')
    icons = parse_pairs(icons_s, 'str')
    noncrit = parse_pairs(noncrit_s, 'int')
    dst = Path(__file__).resolve().parent.parent / 'content' / 'skill-overrides.json'
    payload = {'skill_names': names, 'skill_icons': icons, 'non_critable': noncrit}
    dst.write_text(json.dumps(payload, ensure_ascii=False, indent=0), encoding='utf-8')
    print(f'wrote {dst}: names={len(names)} icons={len(icons)} non_critable={len(noncrit)}')


if __name__ == '__main__':
    main()
