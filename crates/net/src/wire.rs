//! Little-endian byte writer/reader for the wire format. Readers never panic
//! on malformed input: every read returns `None` past the end.

#[derive(Debug, Default, Clone)]
pub struct Writer {
    pub buf: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    pub fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn i32(&mut self, v: i32) {
        self.u32(v as u32);
    }
    pub fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }
    /// LEB128 unsigned varint.
    pub fn var(&mut self, mut v: u64) {
        loop {
            let b = (v & 0x7F) as u8;
            v >>= 7;
            if v == 0 {
                self.u8(b);
                return;
            }
            self.u8(b | 0x80);
        }
    }
    /// Zig-zag signed varint.
    pub fn svar(&mut self, v: i64) {
        self.var(((v << 1) ^ (v >> 63)) as u64);
    }
    pub fn bytes(&mut self, b: &[u8]) {
        self.var(b.len() as u64);
        self.buf.extend_from_slice(b);
    }
    pub fn raw(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }
    pub fn finish(self) -> Vec<u8> {
        self.buf
    }
}

#[derive(Debug, Clone)]
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }
    pub fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.data.get(self.pos..self.pos.checked_add(n)?)?;
        self.pos += n;
        Some(s)
    }
    pub fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    pub fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    pub fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    pub fn i32(&mut self) -> Option<i32> {
        Some(self.u32()? as i32)
    }
    pub fn f32(&mut self) -> Option<f32> {
        Some(f32::from_bits(self.u32()?))
    }
    pub fn var(&mut self) -> Option<u64> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let b = self.u8()?;
            v |= ((b & 0x7F) as u64) << shift;
            if b & 0x80 == 0 {
                return Some(v);
            }
        }
        None
    }
    pub fn svar(&mut self) -> Option<i64> {
        let u = self.var()?;
        Some(((u >> 1) as i64) ^ -((u & 1) as i64))
    }
    pub fn bytes(&mut self) -> Option<&'a [u8]> {
        let n = usize::try_from(self.var()?).ok()?;
        self.take(n)
    }
    pub fn rest(&mut self) -> &'a [u8] {
        let s = &self.data[self.pos..];
        self.pos = self.data.len();
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_all_types() {
        let mut w = Writer::new();
        w.u8(7);
        w.u16(0xBEEF);
        w.u32(0xDEAD_BEEF);
        w.u64(u64::MAX - 3);
        w.i32(-5);
        w.f32(-0.0);
        for v in [0, 1, 127, 128, 300, u64::MAX] {
            w.var(v);
        }
        for v in [0, -1, 1, -64, 64, i64::MIN, i64::MAX] {
            w.svar(v);
        }
        w.bytes(b"hello");
        w.raw(b"tail");
        let buf = w.finish();
        let mut r = Reader::new(&buf);
        assert_eq!(r.u8(), Some(7));
        assert_eq!(r.u16(), Some(0xBEEF));
        assert_eq!(r.u32(), Some(0xDEAD_BEEF));
        assert_eq!(r.u64(), Some(u64::MAX - 3));
        assert_eq!(r.i32(), Some(-5));
        assert_eq!(r.f32().map(f32::to_bits), Some((-0.0f32).to_bits()));
        for v in [0, 1, 127, 128, 300, u64::MAX] {
            assert_eq!(r.var(), Some(v));
        }
        for v in [0, -1, 1, -64, 64, i64::MIN, i64::MAX] {
            assert_eq!(r.svar(), Some(v));
        }
        assert_eq!(r.bytes(), Some(&b"hello"[..]));
        assert_eq!(r.rest(), b"tail");
        assert_eq!(r.remaining(), 0);
    }

    #[test]
    fn truncated_input_is_none_not_panic() {
        let mut r = Reader::new(&[0x80, 0x80]);
        assert_eq!(r.var(), None);
        let mut r = Reader::new(&[5, 1, 2]);
        assert_eq!(r.bytes(), None);
        let mut r = Reader::new(&[1]);
        assert_eq!(r.u32(), None);
        // Absurd length prefix.
        let mut w = Writer::new();
        w.var(u64::MAX);
        let b = w.finish();
        assert_eq!(Reader::new(&b).bytes(), None);
    }

    #[test]
    fn varint_sizes() {
        let size = |v| {
            let mut w = Writer::new();
            w.var(v);
            w.buf.len()
        };
        assert_eq!(size(127), 1);
        assert_eq!(size(128), 2);
        assert_eq!(size(16383), 2);
        assert_eq!(size(u64::MAX), 10);
    }
}
