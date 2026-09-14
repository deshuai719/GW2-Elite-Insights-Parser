# rust/tools — 资产抽取脚本

`rust/content/` 下的资产分两类。

**已入库**(随仓库分发,约 10MB,无对应再生脚本):

| 文件 | 来源 |
|---|---|
| `SkillList.json` | GW2 API 缓存,原 EI CLI publish 产物 |
| `SpecList.json` | 同上 |
| `MapList.json` | 同上 |

**不入库**(`.gitignore` 忽略,约 1.4MB,由下列脚本从 C# 源码重生成):
`buff-table.json`、`skill-overrides.json`、`instant-cast-finders.json`、`icons.json`。

## C# 快照依赖

这些脚本是正则驱动的 C# 源码抽取器:依赖全树 `rglob('*.cs')` 的常量解析、
`ProfHelpers` 的文件名与注册顺序,以及 `SkillItemOverrides.cs` 的字典文本格式。
C# 实现已从工作树移除(本仓库只保留 Rust 版),重新生成资产前需先取回快照:

```bash
git checkout master -- GW2EI.Library
# ...运行下列脚本...
git restore --staged --worktree GW2EI.Library   # 清理,避免误提交
```

源码中约 857 处指向 C# 的溯源注释(如 `CombatData.cs:450-451`)同属悬空引用,
保留为溯源信息,不参与文件读取。

## 用法

```bash
# 在 fork 仓库根下(参数 = GW2EIEvtcParser 目录)
python rust/tools/extract-skill-overrides.py GW2EI.Library/GW2EI.Services/GW2EIEvtcParser
python rust/tools/extract-buff-table.py     GW2EI.Library/GW2EI.Services/GW2EIEvtcParser
# instant-cast finder 表(P3b;参数 = fork 仓库根,与 rust/ 同级,可省略走默认)
python rust/tools/extract-instant-casts.py
# 图标表 prof/target/minion(参数 = fork 仓库根,可省略走默认)
python rust/tools/extract-icons.py
```

脚本输入为 C# 源码快照(当前锚定 `3b7278f9b`);上游更新后重跑可刷新。
`extract-instant-casts.py` 解析 ProfHelpers 的 `InstantCastFinder` 表(通用表 +
按日志 spec 出现的 Helper;`HELPERS` 字典为抽取清单,扩展职业时在此追加),
常量解析 SkillIDs/EffectGUIDs/GW2Builds/SpeciesIDs;lambda checker 结构化翻译
进 `LambdaCond`,无法翻译的条目报 `unresolved`(0 时才是干净状态)。
`extract-icons.py` 解析 `ParserIcons.cs` 与 `SpeciesIDs.cs`,枚举解析不完整时非零退出。

`SkillList/SpecList/MapList.json` 没有对应脚本,已入库;仅在需要刷新 GW2 API 缓存时
从上游 EI 发布产物(`GW2EIParserCommons/Content/*.json`)拷贝后再核对入库。
golden 基准 `rust/golden/*.json` 由 EI 官方 CLI 生成(`rust/tools/ei-golden.conf`),
不入库;缺失时对拍测试自动 skip。
