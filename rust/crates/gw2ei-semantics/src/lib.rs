//! gw2ei-semantics —— EI `EIData/Buffs` 语义层（P3）。
//!
//! 范围（快照 `3b7278f9b`）：
//! - `sim`：BuffSimulator NoID 家族仿真核心（BuffSimulators/NoID）。
//! - `stats`：BuffDistribution / BuffStatistics / BuffByActorStatistics /
//!   BuffGraph 的数值面（公式与统计层）。
//!
//! 不做（本 crate 无关或后续阶段）：BuffSimulatorID（死代码）、
//! CooldownFixer、BuffSourceFinders（pipeline 侧）、ProfHelpers。

pub mod sim;
pub mod stats;
