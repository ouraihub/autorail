//! Bounded JSONL framing and OMP protocol-v2 `rpc_chunk` reassembly.

use std::io::BufRead;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::Value;
use thinkrail_core::{Error, Result};

/// Largest logical frame any engine may send (OMP v2 reassembly ceiling).
pub(crate) const MAX_FRAME: usize = 64 * 1024 * 1024;

/// Reads one newline-terminated JSON frame. `Ok(None)` is a clean EOF; a frame longer than
/// `limit` (newline included) or an unterminated trailing line is a protocol error.
pub(crate) fn read_frame(reader: &mut impl BufRead, limit: usize) -> Result<Option<Value>> {
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf().map_err(|e| Error::protocol(format!("读取 Agent 输出失败：{e}")))?;
        if available.is_empty() {
            if line.is_empty() {
                return Ok(None);
            }
            return Err(Error::protocol("Agent 输出在帧中途结束"));
        }
        let (take, done) = match available.iter().position(|&b| b == b'\n') {
            Some(index) => (index + 1, true),
            None => (available.len(), false),
        };
        if line.len() + take > limit {
            return Err(Error::protocol("Agent 输出的单帧超过上限"));
        }
        line.extend_from_slice(&available[..take]);
        reader.consume(take);
        if done {
            break;
        }
    }
    let text = line.strip_suffix(b"\n").unwrap_or(&line);
    let text = text.strip_suffix(b"\r").unwrap_or(text);
    if text.iter().all(u8::is_ascii_whitespace) {
        return read_frame(reader, limit);
    }
    serde_json::from_slice(text).map(Some).map_err(|e| Error::protocol(format!("Agent 输出不是合法 JSON：{e}")))
}

struct Pending {
    id: String,
    count: u64,
    length: usize,
    next: u64,
    bytes: Vec<u8>,
}

/// Reassembles OMP v2 chunk sequences. Chunks of one id must arrive contiguously and in order.
#[derive(Default)]
pub(crate) struct Chunks {
    pending: Option<Pending>,
}

impl Chunks {
    /// Returns the logical frame: `frame` itself when it is not a chunk, `None` mid-sequence.
    pub(crate) fn accept(&mut self, frame: Value) -> Result<Option<Value>> {
        if frame["type"] != "rpc_chunk" {
            if self.pending.is_some() {
                return Err(Error::protocol("OMP 分片序列被其他帧打断"));
            }
            return Ok(Some(frame));
        }
        let id = frame["chunkId"].as_str().filter(|s| !s.is_empty()).ok_or_else(|| bad("缺少 chunkId"))?;
        let index = frame["index"].as_u64().ok_or_else(|| bad("缺少 index"))?;
        let count = frame["count"].as_u64().filter(|&c| c > 0).ok_or_else(|| bad("count 无效"))?;
        let length = frame["byteLength"]
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|&n| n <= MAX_FRAME)
            .ok_or_else(|| bad("byteLength 无效或超过 64 MiB"))?;
        let data = frame["data"].as_str().filter(|s| !s.is_empty()).ok_or_else(|| bad("缺少 data"))?;
        let data = STANDARD.decode(data).map_err(|_| bad("data 不是合法 base64"))?;
        let pending = match self.pending.take() {
            None if index == 0 => Pending { id: id.into(), count, length, next: 0, bytes: Vec::with_capacity(length) },
            Some(p) if p.id == id && p.count == count && p.length == length && p.next == index => p,
            _ => return Err(bad("分片顺序或元数据不连续")),
        };
        let mut pending = pending;
        if pending.bytes.len() + data.len() > pending.length {
            return Err(bad("分片内容超过声明长度"));
        }
        pending.bytes.extend_from_slice(&data);
        pending.next += 1;
        if pending.next < pending.count {
            self.pending = Some(pending);
            return Ok(None);
        }
        if pending.bytes.len() != pending.length {
            return Err(bad("分片总长度与声明不一致"));
        }
        serde_json::from_slice(&pending.bytes).map(Some).map_err(|e| bad(format!("重组结果不是合法 JSON：{e}")))
    }
}

fn bad(why: impl std::fmt::Display) -> Error {
    Error::protocol(format!("OMP 分片错误：{why}"))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use serde_json::json;
    use thinkrail_core::ErrorKind;

    use super::*;

    fn chunk(id: &str, index: u64, count: u64, length: usize, data: &[u8]) -> Value {
        json!({"type":"rpc_chunk","chunkId":id,"index":index,"count":count,"byteLength":length,"data":STANDARD.encode(data)})
    }

    #[test]
    fn frames_are_bounded_and_eof_is_distinguished() {
        let mut input = Cursor::new(b"{\"a\":1}\r\n\n{\"b\":2}\n".to_vec());
        assert_eq!(read_frame(&mut input, 64).unwrap(), Some(json!({"a":1})));
        assert_eq!(read_frame(&mut input, 64).unwrap(), Some(json!({"b":2})));
        assert_eq!(read_frame(&mut input, 64).unwrap(), None);

        let mut big = Cursor::new(format!("{{\"x\":\"{}\"}}\n", "y".repeat(100)).into_bytes());
        assert_eq!(read_frame(&mut big, 64).unwrap_err().kind(), Some(ErrorKind::Protocol));
        let mut cut = Cursor::new(b"{\"a\":".to_vec());
        assert_eq!(read_frame(&mut cut, 64).unwrap_err().kind(), Some(ErrorKind::Protocol));
    }

    #[test]
    fn chunks_reassemble_in_order_and_reject_gaps_and_lies() {
        let payload = br#"{"type":"message_end","n":1}"#;
        let (a, b) = payload.split_at(10);
        let mut chunks = Chunks::default();
        assert_eq!(chunks.accept(chunk("c", 0, 2, payload.len(), a)).unwrap(), None);
        assert_eq!(chunks.accept(chunk("c", 1, 2, payload.len(), b)).unwrap(), Some(json!({"type":"message_end","n":1})));
        assert_eq!(chunks.accept(json!({"type":"ready"})).unwrap(), Some(json!({"type":"ready"})));

        let mut skipped = Chunks::default();
        assert!(skipped.accept(chunk("c", 1, 2, payload.len(), b)).is_err());
        let mut interrupted = Chunks::default();
        interrupted.accept(chunk("c", 0, 2, payload.len(), a)).unwrap();
        assert!(interrupted.accept(json!({"type":"agent_start"})).is_err());
        let mut short = Chunks::default();
        assert!(short.accept(chunk("c", 0, 1, payload.len() + 1, payload)).is_err());
        let mut long = Chunks::default();
        assert!(long.accept(chunk("c", 0, 1, 3, payload)).is_err());
        let mut base64 = Chunks::default();
        let mut invalid = chunk("c", 0, 1, 1, b"x");
        invalid["data"] = json!("!!!");
        assert!(base64.accept(invalid).is_err());
    }
}
