# GW2 Elite Insights Parser — Rust 实现

[baaron4/GW2-Elite-Insights-Parser](https://github.com/baaron4/GW2-Elite-Insights-Parser) 的 fork，本分支只保留 **Rust 重写**。

原 C# 实现已从工作树移除，完整保留在 **`master`** 分支：

```bash
git checkout master -- GW2EI.Library   # 取回指定路径
git switch master                       # 或整体切到 C# 版
```

> 原版的功能说明、安装步骤、CLI 参数、设置项与 HTML 报告结构等文档，见 `master` 分支的 README。

## 结构

Rust 工作区位于 `rust/`：

| crate | 职责 |
|---|---|
| `gw2ei-parse` | EVTC/zevtc 二进制解析：容器读取、header 解码（rev0/rev1）、agent/skill/combat 列表解码 |
| `gw2ei-model` | 原始 EVTC 之上的领域模型：类型化 combat event 与 `CombatData` 聚合 |
| `gw2ei-semantics` | `EIData/Buffs` 语义层：BuffSimulator NoID 家族仿真核心 |
| `gw2ei-json` | EI JSON 输出契约：serde DTO + 构建器（对齐官方 CLI 输出形状） |
| `gw2ei-cli` | 命令行入口 |

## 构建与测试

```bash
cd rust
cargo build --workspace
cargo test --workspace
```

## CLI

```bash
gw2ei-cli <log.zevtc|log.evtc> [-o out.json] [--content-dir DIR]
```

`--content-dir` 默认为 `content`（相对当前工作目录，约定在 `rust/` 下运行，即指向 `rust/content/`）。

## 内容资产

`rust/content/` 下的资产分两类：

- **已入库**（约 10 MB）：`SkillList.json`、`SpecList.json`、`MapList.json` —— GW2 API 缓存，没有对应的再生脚本，随仓库分发。
- **不入库**：`buff-table.json`、`skill-overrides.json`、`instant-cast-finders.json`、`icons.json` —— 由 `rust/tools/extract-*.py` 从 C# 源码抽取生成。

这些抽取脚本是正则驱动的 C# 源码解析器，依赖全树 `rglob('*.cs')` 的常量解析与 `ProfHelpers` 的注册顺序。重新生成资产前需先取回 C# 快照：

```bash
git checkout master -- GW2EI.Library
# ...运行 rust/tools/ 下的脚本...
git restore --staged --worktree GW2EI.Library
```

脚本用法与产物清单详见 `rust/tools/README.md`。

## 上游

- 原项目：<https://github.com/baaron4/GW2-Elite-Insights-Parser>
- JSON 文档：<https://baaron4.github.io/GW2-Elite-Insights-Parser/Json/index.html>
- Discord：<https://discord.gg/T4kSbKJ5Sf>
