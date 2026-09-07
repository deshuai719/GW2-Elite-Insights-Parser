//! C# System.Text.Json 风格的数值序列化。
//!
//! C# 输出 double 的最短往返且整数值不带小数点(golden 实测 `0` 非 `0.0`);
//! NaN/±Infinity 输出字符串 `"NaN"`/`"Infinity"`/`"-Infinity"`
//!（SerializerSettings.NumberHandling=AllowNamedFloatingPointLiterals）。
//! serde_json 默认 f64 序列化会保留 `.0`,NaN 直接报错 —— 全部 f64 字段经本
//! 模块输出以对齐 golden 的 token 形状（对拍为 Value 级比较,token 类型也须
//! 一致:`0` 整数 token vs `0.0` 浮点 token 在 serde_json::Value 不相等）。

use serde::ser::SerializeSeq;
use serde::{Serialize, Serializer};

/// C# float 字段序列化：STJ 输出 float 的**最短往返十进制**（f32 语义，如
/// `21.821` 而非其精确值 `21.820999145507812`）。Rust f32 走 serde_json
/// 会先扩成 f64 全精度 —— 这里先取最短 f32 表示的 f64 再输出。
pub fn ser_f32<S>(v: &f32, s: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let short: f64 = format!("{v}").parse().unwrap_or_else(|_| f64::from(*v));
    ser_f64(&short, s)
}

/// `Vec<f32>` 平面序列化（元素逐个 ser_f32）。
pub fn ser_f32_list<S>(v: &[f32], s: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let mut seq = s.serialize_seq(Some(v.len()))?;
    for x in v {
        seq.serialize_element(&F32Short(*x))?;
    }
    seq.end()
}

/// `Vec<f32>` 元素的 short 表示代理。
struct F32Short(f32);

impl Serialize for F32Short {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        ser_f32(&self.0, s)
    }
}

/// `&[f32]` 代理（内嵌列表）。
struct F32List<'a>(&'a [f32]);

impl Serialize for F32List<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        ser_f32_list(self.0, s)
    }
}

/// `Vec<Vec<f32>>`（[x, y] 对）序列化。
pub fn ser_f32_pairs<S>(v: &[Vec<f32>], s: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let mut seq = s.serialize_seq(Some(v.len()))?;
    for pair in v {
        seq.serialize_element(&F32List(pair))?;
    }
    seq.end()
}

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
