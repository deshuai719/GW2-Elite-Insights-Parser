//! 事件化错误。P1 只支持新世代（ArcDPS build >= 20260501，
//! `BuffAppliesAndRemovesAsStateChanges` + `ResultEnumRework`）日志。
//! 旧世代日志在 `factory` 入口显式报错（不静默降级），语义对齐点：
//! 主分发四路判定（CombatItem.cs:214-431 新世代分支）之外的旧世代路径
//! 全部走这里暴露。

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelError {
    /// 日志 arc build < 20260501：事件行语义是旧世代（buff apply/remove
    /// 无独立 state change、cast 用 Activation、DamageResult 未引入），
    /// P1 工厂不支持，返回显式错误。
    UnsupportedArcBuild { build: i32 },
    /// `CompletePlayers` 找不到任何 AgentType::Player（EvtcParser.cs:1314-1317）。
    NoPlayersFound,
    /// `CompletePlayers` 过滤后玩家列表为空（EvtcParser.cs:1053-1056）。
    NoValidPlayers,
    /// P2a 只支持 WvW 识别路径；非 WvW 的 encounter（45 boss 表未迁移）
    /// 显式报错（EvtcParser.cs:111-342 的 DetectLogic 其余分支，P4）。
    UnsupportedEncounter { trigger_id: u16 },
    /// 事件时间轴之外/无法归因的结构性错误（链接阶段自检）。
    InconsistentAgentState(&'static str),
    /// 二进制解析层错误（gw2ei-parse）。
    ParseError(String),
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModelError::UnsupportedArcBuild { build } => write!(
                f,
                "ArcDPS build {build} is below 20260501 (BuffAppliesAndRemovesAsStateChanges + ResultEnumRework); only the new-generation event layout is supported in stage P1"
            ),
            ModelError::NoPlayersFound => write!(f, "No players found"),
            ModelError::NoValidPlayers => write!(f, "No valid players"),
            ModelError::UnsupportedEncounter { trigger_id } => write!(
                f,
                "trigger id {trigger_id} does not resolve to the WvW path; boss/instance encounter logic is not migrated in stage P2a"
            ),
            ModelError::InconsistentAgentState(msg) => write!(f, "inconsistent agent state: {msg}"),
            ModelError::ParseError(msg) => write!(f, "parse error: {msg}"),
        }
    }
}

impl std::error::Error for ModelError {}
