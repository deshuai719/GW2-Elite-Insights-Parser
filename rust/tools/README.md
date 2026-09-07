# rust/tools — 资产抽取脚本

`rust/content/`(SkillList/SpecList/MapList/buff-table/skill-overrides,约 12MB)不入库
(.gitignore),由下列脚本从 EI fork 仓库(C# 源码)重生成:

```
# 在 EI fork 仓库根下(脚本参数 = GW2EIEvtcParser 目录)
python rust/tools/extract-skill-overrides.py GW2EI.Library/GW2EI.Services/GW2EIEvtcParser
python rust/tools/extract-buff-table.py     GW2EI.Library/GW2EI.Services/GW2EIEvtcParser
# SkillList/SpecList/MapList.json:EI CLI publish 产物自带
# (GW2EIParserCommons/Content/*.json),直接拷贝
```

脚本输入为 C# 源码快照(当前锚定 `3b7278f9b`);上游更新后重跑可刷新。
golden 基准 `rust/golden/*.json` 由 EI 官方 CLI 生成(`rust/tools/ei-golden.conf`),
不入库;缺失时对拍测试自动 skip。
