//! 事件化错误。P1 只支持新世代（ArcDPS build >= 20260501，
//! `BuffAppliesAndRemovesAsStateChanges` + `ResultEnumRework`）日志。
//! 旧世代日志在 `factory` 入口显式报错（不静默降级），语义对齐点：
//! 主分发四路判定（CombatItem.cs:214-431 新世代分支）之外的旧世代路径
//! 全部走这里暴露。

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelError {
    /// 日志 arc build < 20260501：事件行语义是旧世代（buff apply/remove
    /// 无独立 state change、cast 用 Activation、DamageResult 未引入），
    /// P1 工厂不支持，返回显式错误。
    UnsupportedArcBuild { build: i32 },
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModelError::UnsupportedArcBuild { build } => write!(
                f,
                "ArcDPS build {build} is below 20260501 (BuffAppliesAndRemovesAsStateChanges + ResultEnumRework); only the new-generation event layout is supported in stage P1"
            ),
        }
    }
}

impl std::error::Error for ModelError {}
