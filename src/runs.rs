//! Run-length coded byte rasters. Browser-free.
//!
//! The raster worker (`assets/raster-worker.js`, `encodeRuns`) sends the
//! land and glacier masks this way: they are mostly long runs of 0 and 255
//! with antialiased edges, so the 8192-wide land mask (32 MB raw) codes to
//! a few MB. The app keeps the coded bytes (for a context restore, or to
//! put the modern map back after deep time) and decodes them a strip at a
//! time into a small buffer for upload, so the raw raster never sits in
//! memory whole.
//!
//! Format: a sequence of runs, each a value byte followed by the run length
//! as an unsigned LEB128 varint (7 bits a byte, low bits first, high bit
//! set on every byte but the last). Lengths are at least 1. Runs may span
//! rows.

/// The decoded length of `bytes`, checking that they are well formed.
pub fn decoded_len(bytes: &[u8]) -> Result<usize, String> {
    let mut r = Reader::new(bytes);
    let mut total = 0usize;
    while r.pos < bytes.len() {
        r.next_run()?;
        total = total.checked_add(r.left).ok_or("run lengths overflow")?;
        r.left = 0;
    }
    Ok(total)
}

/// Decodes runs into consecutive output slices.
pub struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
    value: u8,
    /// Bytes of the current run not yet written.
    left: usize,
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Reader {
            bytes,
            pos: 0,
            value: 0,
            left: 0,
        }
    }

    fn next_run(&mut self) -> Result<(), String> {
        let value = *self.bytes.get(self.pos).ok_or("truncated run")?;
        self.pos += 1;
        let mut len = 0u64;
        let mut shift = 0u32;
        loop {
            let b = *self.bytes.get(self.pos).ok_or("truncated run length")?;
            self.pos += 1;
            if shift > 28 {
                return Err("run length too long".into());
            }
            len |= u64::from(b & 0x7f) << shift;
            shift += 7;
            if b & 0x80 == 0 {
                break;
            }
        }
        let len = usize::try_from(len).map_err(|_| "run length too long")?;
        if len == 0 {
            return Err("empty run".into());
        }
        self.value = value;
        self.left = len;
        Ok(())
    }

    /// Fill `out` with the next `out.len()` decoded bytes.
    pub fn fill(&mut self, out: &mut [u8]) -> Result<(), String> {
        let mut i = 0;
        while i < out.len() {
            if self.left == 0 {
                self.next_run()?;
            }
            let n = self.left.min(out.len() - i);
            out[i..i + n].fill(self.value);
            self.left -= n;
            i += n;
        }
        Ok(())
    }
}

/// Code `src` as runs; the reference for `encodeRuns` in the worker.
#[cfg(test)]
pub fn encode(src: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < src.len() {
        let v = src[i];
        let mut j = i + 1;
        while j < src.len() && src[j] == v {
            j += 1;
        }
        out.push(v);
        let mut len = j - i;
        while len >= 0x80 {
            out.push((len & 0x7f) as u8 | 0x80);
            len >>= 7;
        }
        out.push(len as u8);
        i = j;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_in_strips() {
        let mut src = vec![0u8; 100_000];
        src[40_000..70_000].fill(255);
        src[39_999] = 128;
        src[70_000] = 3;
        src.extend((0..=255u8).cycle().take(5000));
        let coded = encode(&src);
        assert!(coded.len() < 11_000, "{}", coded.len());
        assert_eq!(decoded_len(&coded), Ok(src.len()));
        let mut r = Reader::new(&coded);
        let mut out = Vec::new();
        for strip in [1usize, 7, 39_991, 30_002, 1, 34_998] {
            let mut buf = vec![9u8; strip];
            r.fill(&mut buf).unwrap();
            out.extend(buf);
        }
        assert_eq!(out, src);
        // Past the end.
        assert!(r.fill(&mut [0u8; 1]).is_err());
    }

    #[test]
    fn long_runs_use_multi_byte_lengths() {
        let src = vec![255u8; 16 * 1024 * 1024];
        let coded = encode(&src);
        assert_eq!(coded, vec![255, 0x80, 0x80, 0x80, 0x08]);
        assert_eq!(decoded_len(&coded), Ok(src.len()));
    }

    #[test]
    fn malformed_input_is_an_error() {
        assert_eq!(decoded_len(&[]), Ok(0));
        assert!(decoded_len(&[5]).is_err(), "no length");
        assert!(decoded_len(&[5, 0x80]).is_err(), "truncated length");
        assert!(decoded_len(&[5, 0]).is_err(), "empty run");
        assert!(
            decoded_len(&[5, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 1]).is_err()
        );
        let mut r = Reader::new(&[7, 2]);
        let mut out = [0u8; 3];
        assert!(r.fill(&mut out).is_err());
    }
}
