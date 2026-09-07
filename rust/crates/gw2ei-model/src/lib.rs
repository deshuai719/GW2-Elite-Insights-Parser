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

pub mod agent;
mod combat_data;
mod error;
mod manipulation;
pub mod events;
mod factory;
mod parsed;
mod stats;
mod spec;
mod state_change;

pub use agent::{AgentId, AgentItem, AgentMergedFrom, AgentTable, AgentType, NO_AGENT, species};
pub use combat_data::{CategoryCounts, CombatData, MetaDataBucket};
pub use error::ModelError;
pub use events::{AnimationStatus, CombatEvent, UnsupportedEventKind};
pub use factory::{build_combat_data, AgentLookup};
pub use manipulation::{LinkStats, complete_agents};
pub use parsed::{Friend, LogData, NonSquadPlayerActor, ParsedLog, PhaseData, PlayerActor, NpcActor, assemble_raw, assemble_raw_with, parse_log_file, parse_log_file_with};
pub use stats::{DamageStats, DmgRow, EventIndex, Segment, StatsCompleteness, build_event_index, compute_damage_from, compute_dps_stats, damage_graph_1s, dmg_row, is_down_before_next_90, list_from_states, percent_points, status_segments};
pub use spec::{NoSpecCatalog, Spec, SpecCatalog, spec_from_prof_elite};
