//! Binary EVTC/zevtc parsing: container reading, header decoding (rev0/rev1),
//! agent/skill/combat list decoding.
//!
//! Ported from `GW2EIEvtcParser/EvtcParser/EvtcParser.cs` at upstream snapshot
//! `3b7278f9b`. Scope is driven by SkillBuffReplay consumption needs, not a
//! full 1:1 translation; see `.trellis/tasks/09-07-rust-rewrite-ei/` in the
//! SkillBuffReplay repository for the staged plan.
//!
//! 分层定位：本 crate 完成 `ParseLogData → ParseAgentData → ParseSkillData →
//! ParseCombatList`（EvtcParser.cs:485-914），产出过滤 + 时间归零后的
//! combat 行；`CompleteAgentsAndLogData`/`PreProcessEvtcData`（P2）与事件
//! 类型化（`gw2ei-model`）在此之上进行。

mod enums;
mod error;
mod log_reader;
mod models;
mod reader;
mod zip_reader;

pub use enums::arc_builds;
pub use enums::{
    Activation, AnimationStart, AnimationStop, BreakbarState, BuffCycle, BuffRemove,
    BuffStackType, ConditionResult, ContentLocal, DamageResult, Iff, Language, LogType,
    SkillAction, SquadMarkerIndex, StateChange,
};
pub use error::EvtcError;
pub use log_reader::{parse_evtc_log, parse_evtc_log_with_settings, read_evtc_log};
pub use models::{
    DiscardStats, EvtcCombatItem, EvtcRawAgent, EvtcRawLog, EvtcRawSkill, PadBytes,
    ParseStats, ParserSettings, RawAgentKind,
};
pub use models::skill_ids;
