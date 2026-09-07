//! Domain model over raw EVTC data: typed combat events and CombatData
//! aggregation.
//!
//! Ported from `GW2EIEvtcParser/ParsedData/CombatEvents/` (typed event tree)
//! and `CombatData.cs` / `CombatDataFetchers.cs` at upstream snapshot
//! `3b7278f9b`. Scope is driven by SkillBuffReplay consumption needs, not a
//! full 1:1 translation; see `.trellis/tasks/09-07-rust-rewrite-ei/` in the
//! SkillBuffReplay repository for the staged plan.
//!
//! P1 范围：新世代（arc build ≥ 20260501）分发路径的事件类型化；
//! 旧世代入口报 `ModelError::UnsupportedArcBuild`。H 组事件（effect/marker/
//! missile 等）显式计数不静默丢弃。

mod combat_data;
mod error;
mod events;
mod factory;
mod spec;
mod state_change;

pub use combat_data::{CategoryCounts, CombatData, MetaDataBucket};
pub use error::ModelError;
pub use events::{AnimationStatus, CombatEvent, UnsupportedEventKind};
pub use factory::{build_combat_data, AgentLookup};
pub use spec::{Spec, spec_from_prof_elite};
