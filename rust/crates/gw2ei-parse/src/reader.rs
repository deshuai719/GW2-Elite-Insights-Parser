//! 小端切片读取器，对齐 C# `EvtcParser` 直接顺序消费 `BinaryReader` 的行为。
//! 全程 safe：切片 + `from_le_bytes`，不做零拷贝指针解引用（C# 原库的
//! `AllowUnsafeBlocks` 只用于零拷贝，此处用切片等价）。

use crate::error::EvtcError;

pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    pub(crate) fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }

    fn ensure_available(&self, byte_count: usize) -> Result<(), EvtcError> {
        if self.remaining() < byte_count {
            return Err(EvtcError::UnexpectedEnd {
                position: self.position,
                required: byte_count,
                remaining: self.remaining(),
            });
        }
        Ok(())
    }

    fn take(&mut self, byte_count: usize) -> &'a [u8] {
        let start = self.position;
        self.position += byte_count;
        &self.bytes[start..start + byte_count]
    }

    /// 取定长数组（ensure_available 保证可读；copy_from_slice 等长无 panic）。
    fn take_array<const N: usize>(&mut self) -> [u8; N] {
        let start = self.position;
        self.position += N;
        let mut out = [0u8; N];
        out.copy_from_slice(&self.bytes[start..start + N]);
        out
    }

    pub(crate) fn read_u8(&mut self) -> Result<u8, EvtcError> {
        self.ensure_available(1)?;
        Ok(self.take(1)[0])
    }

    pub(crate) fn read_u16(&mut self) -> Result<u16, EvtcError> {
        self.ensure_available(2)?;
        Ok(u16::from_le_bytes(self.take_array()))
    }

    pub(crate) fn read_i32(&mut self) -> Result<i32, EvtcError> {
        self.ensure_available(4)?;
        Ok(i32::from_le_bytes(self.take_array()))
    }

    pub(crate) fn read_u32(&mut self) -> Result<u32, EvtcError> {
        self.ensure_available(4)?;
        Ok(u32::from_le_bytes(self.take_array()))
    }

    pub(crate) fn read_i64(&mut self) -> Result<i64, EvtcError> {
        self.ensure_available(8)?;
        Ok(i64::from_le_bytes(self.take_array()))
    }

    pub(crate) fn read_u64(&mut self) -> Result<u64, EvtcError> {
        self.ensure_available(8)?;
        Ok(u64::from_le_bytes(self.take_array()))
    }

    /// 取定长字节切片（不做 UTF-8 解码，解码归调用方）。
    pub(crate) fn take_bytes(&mut self, byte_count: usize) -> Result<&'a [u8], EvtcError> {
        self.ensure_available(byte_count)?;
        Ok(self.take(byte_count))
    }

    pub(crate) fn skip(&mut self, byte_count: usize) -> Result<(), EvtcError> {
        self.ensure_available(byte_count)?;
        self.position += byte_count;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_little_endian_fields() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1u8.to_le_bytes());
        bytes.extend_from_slice(&0x1234u16.to_le_bytes());
        bytes.extend_from_slice(&(-2i32).to_le_bytes());
        bytes.extend_from_slice(&0xdeadbeefu32.to_le_bytes());
        bytes.extend_from_slice(&(-9i64).to_le_bytes());
        bytes.extend_from_slice(&0x1122334455667788u64.to_le_bytes());
        let mut r = Reader::new(&bytes);
        assert_eq!(r.read_u8().expect("synthetic test bytes must parse"), 1);
        assert_eq!(r.read_u16().expect("synthetic test bytes must parse"), 0x1234);
        assert_eq!(r.read_i32().expect("synthetic test bytes must parse"), -2);
        assert_eq!(r.read_u32().expect("synthetic test bytes must parse"), 0xdeadbeef);
        assert_eq!(r.read_i64().expect("synthetic test bytes must parse"), -9);
        assert_eq!(r.read_u64().expect("synthetic test bytes must parse"), 0x1122334455667788);
        assert_eq!(r.remaining(), 0);
    }

    #[test]
    fn unexpected_end_reports_position() {
        let mut r = Reader::new(&[0u8; 3]);
        let err = r.read_u64().expect_err("synthetic bytes must fail");
        match err {
            EvtcError::UnexpectedEnd {
                position,
                required,
                remaining,
            } => {
                assert_eq!(position, 0);
                assert_eq!(required, 8);
                assert_eq!(remaining, 3);
            }
            other => panic!("unexpected error: {other}"),
        }
        // 部分消费后越界
        let mut r = Reader::new(&[0u8; 5]);
        let _ = r.read_u32().expect("synthetic test bytes must parse");
        assert!(r.read_u16().is_err());
    }
}
