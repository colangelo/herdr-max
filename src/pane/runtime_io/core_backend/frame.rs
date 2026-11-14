//! herdr-core's framed mode (core `schema/README.md` § Framed mode):
//! `[channel u32 LE][kind u8][len u32 LE][payload]`, payload at most 16 MiB.
//! Channel 0 carries JSON-RPC; other channels are attach streams.

/// The largest payload core sends or accepts.
pub(crate) const MAX_PAYLOAD: usize = 16 * 1024 * 1024;

const HEADER_LEN: usize = 9;

/// Frame kinds this client knows. A frame of another kind is kept as
/// `Frame::kind` and ignored by the reader, as core ignores kinds it does not
/// know.
pub(crate) mod kind {
    pub(crate) const DATA: u8 = 0;
    pub(crate) const CREDIT: u8 = 1;
    pub(crate) const CLOSE: u8 = 2;
    pub(crate) const RESIZE: u8 = 3;
    pub(crate) const FACTS: u8 = 5;
    pub(crate) const AUTOMATIC_INPUT: u8 = 6;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Frame {
    pub channel: u32,
    pub kind: u8,
    pub payload: Vec<u8>,
}

impl Frame {
    pub(crate) fn new(channel: u32, kind: u8, payload: impl Into<Vec<u8>>) -> Self {
        Self {
            channel,
            kind,
            payload: payload.into(),
        }
    }

    pub(crate) fn credit(channel: u32, bytes: u32) -> Self {
        Self::new(channel, kind::CREDIT, bytes.to_le_bytes())
    }

    pub(crate) fn resize(channel: u32, cols: u16, rows: u16) -> Self {
        let mut payload = Vec::with_capacity(4);
        payload.extend_from_slice(&cols.to_le_bytes());
        payload.extend_from_slice(&rows.to_le_bytes());
        Self::new(channel, kind::RESIZE, payload)
    }

    /// Appends the encoded frame to `out`.
    pub(crate) fn encode_into(&self, out: &mut Vec<u8>) -> Result<(), FrameError> {
        if self.payload.len() > MAX_PAYLOAD {
            return Err(FrameError::TooLarge(self.payload.len()));
        }
        out.reserve(HEADER_LEN + self.payload.len());
        out.extend_from_slice(&self.channel.to_le_bytes());
        out.push(self.kind);
        // MAX_PAYLOAD fits in u32, checked above.
        out.extend_from_slice(&(self.payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.payload);
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum FrameError {
    /// A payload over 16 MiB: the stream is corrupt or not framed.
    TooLarge(usize),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::TooLarge(len) => {
                write!(f, "herdr-core frame payload of {len} bytes is over 16 MiB")
            }
        }
    }
}

impl std::error::Error for FrameError {}

/// Reassembles frames from arbitrary reads.
#[derive(Default)]
pub(crate) struct FrameDecoder {
    buf: Vec<u8>,
    start: usize,
}

impl FrameDecoder {
    pub(crate) fn push(&mut self, bytes: &[u8]) {
        if self.start > 0 && self.start == self.buf.len() {
            self.buf.clear();
            self.start = 0;
        }
        self.buf.extend_from_slice(bytes);
    }

    /// The next complete frame, if one is buffered. An oversized length is an
    /// error as soon as the header arrives, before its payload is buffered.
    pub(crate) fn next_frame(&mut self) -> Result<Option<Frame>, FrameError> {
        let available = &self.buf[self.start..];
        if available.len() < HEADER_LEN {
            return Ok(None);
        }
        let channel = u32::from_le_bytes([available[0], available[1], available[2], available[3]]);
        let kind = available[4];
        let len =
            u32::from_le_bytes([available[5], available[6], available[7], available[8]]) as usize;
        if len > MAX_PAYLOAD {
            return Err(FrameError::TooLarge(len));
        }
        if available.len() < HEADER_LEN + len {
            return Ok(None);
        }
        let payload = available[HEADER_LEN..HEADER_LEN + len].to_vec();
        self.start += HEADER_LEN + len;
        if self.start > 64 * 1024 && self.start * 2 > self.buf.len() {
            self.buf.drain(..self.start);
            self.start = 0;
        }
        Ok(Some(Frame {
            channel,
            kind,
            payload,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(frame: &Frame) -> Vec<u8> {
        let mut out = Vec::new();
        frame.encode_into(&mut out).unwrap();
        out
    }

    #[test]
    fn encodes_the_documented_header_layout() {
        let bytes = encoded(&Frame::new(7, kind::DATA, b"hi".to_vec()));
        assert_eq!(bytes, [7, 0, 0, 0, 0, 2, 0, 0, 0, b'h', b'i']);
    }

    #[test]
    fn credit_and_resize_payloads_are_little_endian() {
        assert_eq!(Frame::credit(1, 0x0102_0304).payload, [4, 3, 2, 1]);
        assert_eq!(Frame::resize(1, 120, 40).payload, [120, 0, 40, 0]);
    }

    #[test]
    fn decodes_frames_split_at_every_byte() {
        let frames = [
            Frame::new(0, kind::DATA, br#"{"jsonrpc":"2.0","id":1}"#.to_vec()),
            Frame::new(3, kind::CLOSE, Vec::new()),
            Frame::credit(3, 4096),
        ];
        let mut wire = Vec::new();
        for frame in &frames {
            frame.encode_into(&mut wire).unwrap();
        }
        let mut decoder = FrameDecoder::default();
        let mut seen = Vec::new();
        for byte in wire {
            decoder.push(&[byte]);
            while let Some(frame) = decoder.next_frame().unwrap() {
                seen.push(frame);
            }
        }
        assert_eq!(seen, frames);
    }

    #[test]
    fn decodes_several_frames_from_one_read() {
        let mut wire = encoded(&Frame::new(2, kind::DATA, b"one".to_vec()));
        wire.extend(encoded(&Frame::new(2, kind::DATA, b"two".to_vec())));
        let mut decoder = FrameDecoder::default();
        decoder.push(&wire);
        assert_eq!(decoder.next_frame().unwrap().unwrap().payload, b"one");
        assert_eq!(decoder.next_frame().unwrap().unwrap().payload, b"two");
        assert_eq!(decoder.next_frame().unwrap(), None);
    }

    #[test]
    fn keeps_unknown_kinds_for_the_reader_to_ignore() {
        let mut decoder = FrameDecoder::default();
        decoder.push(&encoded(&Frame::new(1, 42, b"x".to_vec())));
        assert_eq!(decoder.next_frame().unwrap().unwrap().kind, 42);
    }

    #[test]
    fn refuses_an_oversized_length_from_the_header_alone() {
        let mut header = vec![1, 0, 0, 0, kind::DATA];
        header.extend_from_slice(&((MAX_PAYLOAD as u32) + 1).to_le_bytes());
        let mut decoder = FrameDecoder::default();
        decoder.push(&header);
        assert_eq!(
            decoder.next_frame(),
            Err(FrameError::TooLarge(MAX_PAYLOAD + 1))
        );
    }

    #[test]
    fn refuses_to_encode_an_oversized_payload() {
        let frame = Frame::new(1, kind::DATA, vec![0; MAX_PAYLOAD + 1]);
        assert_eq!(
            frame.encode_into(&mut Vec::new()),
            Err(FrameError::TooLarge(MAX_PAYLOAD + 1))
        );
    }
}
