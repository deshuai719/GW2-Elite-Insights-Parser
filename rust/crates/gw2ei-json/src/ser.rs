//! C# System.Text.Json 风格的数值序列化。
//!
//! C# 输出 double 的最短往返且整数值不带小数点(golden 实测 `0` 非 `0.0`);
//! NaN/±Infinity 输出字符串 `"NaN"`/`"Infinity"`/`"-Infinity"`
//!（SerializerSettings.NumberHandling=AllowNamedFloatingPointLiterals）。
//! serde_json 默认 f64 序列化会保留 `.0`,NaN 直接报错 —— 全部 f64 字段经本
//! 模块输出以对齐 golden 的 token 形状（对拍为 Value 级比较,token 类型也须
//! 一致:`0` 整数 token vs `0.0` 浮点 token 在 serde_json::Value 不相等）。

use serde::Serializer;

/// `f64` 字段序列化:整数值 → 整数 token;NaN/Inf → 字符串;否则最短往返。
pub fn ser_f64<S>(v: &f64, s: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if v.is_nan() {
        return s.serialize_str("NaN");
    }
    if v.is_infinite() {
        return if *v > 0.0 {
            s.serialize_str("Infinity")
        } else {
            s.serialize_str("-Infinity")
        };
    }
    if v.fract() == 0.0 {
        // i64 范围内整值:输出整数 token;范围外走 f64(不出现于本契约)。
        // 上界用 2^63(f64 中 I64_MAX=2^63-1 的表示值恰为 2^63),须严格
        // 小于 —— <= I64_MAX 常量会把 2^63 误判为范围内而饱和成 i64::MAX。
        const I64_MIN: f64 = -9_223_372_036_854_775_808.0; // -2^63,可精确表示
        const I64_MAX_EXCL: f64 = 9_223_372_036_854_775_808.0; // 2^63
        if *v >= I64_MIN && *v < I64_MAX_EXCL {
            return s.serialize_i64(*v as i64);
        }
    }
    s.serialize_f64(*v)
}

#[cfg(test)]
mod tests {
    use serde::Serialize;
    use serde_json::{json, Value};

    use super::ser_f64;

    #[derive(Serialize)]
    struct T {
        #[serde(serialize_with = "ser_f64")]
        v: f64,
    }

    fn to_value(v: f64) -> Value {
        serde_json::to_value(T { v })
            .expect("serialize")
            .get("v")
            .cloned()
            .expect("field v")
    }

    /// golden 样例不含 NaN/Inf 值(该分支无对拍覆盖),此处钉住契约:
    /// C# NumberHandling=AllowNamedFloatingPointLiterals → "NaN"/"Infinity"
    /// /"-Infinity" 字符串;serde_json 默认 f64 NaN 直接报错,故必须经 ser_f64。
    #[test]
    fn named_float_literals() {
        assert_eq!(to_value(f64::NAN), json!("NaN"));
        assert_eq!(to_value(f64::INFINITY), json!("Infinity"));
        assert_eq!(to_value(f64::NEG_INFINITY), json!("-Infinity"));
    }

    /// 整数值输出整数 token(golden 实测 `0` 非 `0.0`;Value 级对拍要求
    /// token 形状一致)。
    #[test]
    fn integral_values_become_int_tokens() {
        assert_eq!(to_value(0.0), json!(0));
        assert_eq!(to_value(-0.0), json!(0));
        assert_eq!(to_value(42.0), json!(42));
        assert_eq!(to_value(-3.0), json!(-3));
    }

    #[test]
    fn fractional_and_large_values_stay_f64() {
        assert_eq!(to_value(0.5), json!(0.5));
        assert_eq!(to_value(-2.25), json!(-2.25));
        // i64 范围外(不出现于契约,仍应输出 f64 而非整型溢出)。
        assert_eq!(
            to_value(9_223_372_036_854_775_808.0),
            json!(9_223_372_036_854_775_808.0f64)
        );
    }
}
