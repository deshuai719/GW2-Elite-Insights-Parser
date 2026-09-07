//! EI JSON 输出契约:serde DTO + 构建器(对齐 GW2EIJSON/JsonLogBuilder 与
//! 官方 CLI 输出形状)。上游快照 `3b7278f9b`;序列化形状的最终事实是
//! `rust/golden/*.json`(EI 官方 CLI 生成,只读对拍)。
//!
//! P2b 范围:WvW 路径(非 WvW 日志由 gw2ei-model 报 UnsupportedEncounter)。
//! 分块验收:IMPLEMENTED 块逐键与 golden 一致;KNOWN_DIFF/OOS 见
//! `tests/golden_compare.rs` 的块清单。

pub mod actors;
pub mod buffsim;
pub mod build;
pub mod content;
pub mod ctx;
pub mod json_buffs;
pub mod dto_a;
pub mod dto_b;
pub mod rows;
pub mod ser;
pub mod skills;
pub mod stats;

pub use content::{BuffClassification, BuffDef, BuffRegistry, Content, load_default_content};
pub use dto_a::JsonLog;
pub use build::build_json;
pub use ctx::{BuildError, BuildOptions, build_report, parse_and_build};

/// skill/buff id 符号扩展:raw 层以 u32 存储(bit 位同 C# int),C# 语义
/// 为 long 带符号(`s" + (long)(int)skillID`;负 id = 合成技能/扩展 buff)。
pub fn signed_id(u: u32) -> i64 {
    i64::from(u as i32)
}

/// 数字规整:C# System.Text.Json 对整数值 double 输出 `0` 而非 `0.0`
/// (最短往返);serde_json f64 恒 `0.0`。输出前把整值 f64 折为 i64,使
/// JSON 文本与 golden 一致(仅 f64 无损值域;大整数原样)。
pub fn shorten_numbers(v: serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Number(n) => {
            // 已是整数 token 的(i64/u64)原样保留 —— 经 f64 中转会在
            // >2^53 时精度丢失,不能先 as_f64。
            if n.as_i64().is_some() || n.as_u64().is_some() {
                return serde_json::Value::Number(n);
            }
            if let Some(f) = n.as_f64()
                && f.is_finite()
                && f.fract() == 0.0
                && f.abs() <= 9_007_199_254_740_992.0
            {
                return serde_json::Value::from(f as i64);
            }
            serde_json::Value::Number(n)
        }
        serde_json::Value::Array(a) => {
            serde_json::Value::Array(a.into_iter().map(shorten_numbers).collect())
        }
        serde_json::Value::Object(o) => serde_json::Value::Object(
            o.into_iter().map(|(k, v)| (k, shorten_numbers(v))).collect(),
        ),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use super::shorten_numbers;

    /// f64 整值折整数 token;已是整数 token 的大数不经 f64 中转
    /// (>2^53 会精度丢失)。
    #[test]
    fn whole_f64_flattened_but_big_ints_untouched() {
        assert_eq!(shorten_numbers(json!(0.0)), json!(0));
        assert_eq!(shorten_numbers(json!(0.5)), json!(0.5));
        assert_eq!(
            shorten_numbers(Value::from(9_007_199_254_740_993i64)),
            json!(9_007_199_254_740_993i64)
        );
        assert_eq!(
            shorten_numbers(Value::from(i64::MAX)),
            json!(i64::MAX)
        );
    }

    #[test]
    fn recurses_into_containers() {
        let v = json!({"a": {"b": [1.0, 2.5]}, "c": 3.0});
        assert_eq!(shorten_numbers(v), json!({"a": {"b": [1, 2.5]}, "c": 3}));
    }
}
