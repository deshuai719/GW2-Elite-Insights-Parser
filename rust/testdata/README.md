# rust/testdata — EI Rust 重构的测试样例

本目录放 EI Rust 工作区的集成测试样例（gitignore 忽略，不入库）。

## 当前样例

| 文件 | 来源 | 说明 |
|---|---|---|
| `20260530-205048.zevtc` | SBR 白名单样例拷贝（SkillBuffReplay 仓库 `testdata/`） | 训练假人日志；410 agents / 740 skills / 320,684 raw combat 行；header `EVTC20260507`、rev1、boss 触发（id=1）、含 1 条 ArcBuild 行（payload `20260507.234558`）、GW2 build 200968 |

## 登记策略

- 新增样例前先在此登记（来源 + 用途），再拷贝文件。
- 推荐来源：SBR 仓库 `testdata/`（已通过 SBR golden 验证的 15 个真实样例）；
  需要覆盖新场景（instance 日志、rev0、effect/marker 行族）时再从 EI 官方
  测试日志或 SBR 白名单补齐。
- 文件较大（4-20MB），不入 git；测试按 skip-if-missing 模式编写，
  CI 无样例自动跳过。
- 目录已加入仓库根 `.gitignore`（`rust/testdata/*.zevtc`）。

## 与 EI 官方 golden 对拍（P2 起）

- 官方基准：`GW2EIParserCLI -c rust/tools/ei-golden.conf <log>` 生成 JSON 到
  `rust/golden/`（该目录同样 gitignore）。
- 对拍测试位于各 crate `tests/`，基准缺失时 skip。
