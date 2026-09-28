//! Enough of zlib's inflate (RFC 1950 and 1951) to read the start of a git
//! object, so the write hook can read one without a `git` process or a new
//! dependency. It stops as soon as it has the bytes asked for.

const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LEN_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
const ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

struct Bits<'a> {
    input: &'a [u8],
    pos: usize,
    buf: u32,
    count: u32,
}

impl Bits<'_> {
    fn take(&mut self, n: u32) -> Option<u32> {
        while self.count < n {
            self.buf |= u32::from(*self.input.get(self.pos)?) << self.count;
            self.pos += 1;
            self.count += 8;
        }
        let v = self.buf & ((1u32 << n) - 1);
        self.buf >>= n;
        self.count -= n;
        Some(v)
    }
}

/// A canonical Huffman code: how many codes of each length, and the
/// symbols in code order.
struct Code {
    count: [u16; 16],
    symbol: Vec<u16>,
}

impl Code {
    fn new(lengths: &[u8]) -> Option<Code> {
        let mut count = [0u16; 16];
        for &l in lengths {
            count[usize::from(l)] += 1;
        }
        let mut left = 1i32;
        for &c in &count[1..] {
            left = (left << 1) - i32::from(c);
            if left < 0 {
                return None;
            }
        }
        let mut offset = [0u16; 16];
        for len in 1..15 {
            offset[len + 1] = offset[len] + count[len];
        }
        let mut symbol = vec![0u16; lengths.len()];
        for (s, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbol[usize::from(offset[usize::from(l)])] = s as u16;
                offset[usize::from(l)] += 1;
            }
        }
        Some(Code { count, symbol })
    }

    fn decode(&self, bits: &mut Bits) -> Option<u16> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for &count in &self.count[1..] {
            code |= bits.take(1)? as i32;
            let count = i32::from(count);
            if code - count < first {
                return self.symbol.get((index + code - first) as usize).copied();
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        None
    }
}

fn fixed() -> Option<(Code, Code)> {
    let mut lengths = [8u8; 288];
    lengths[144..256].fill(9);
    lengths[256..280].fill(7);
    Some((Code::new(&lengths)?, Code::new(&[5; 30])?))
}

fn dynamic(bits: &mut Bits) -> Option<(Code, Code)> {
    let nlen = bits.take(5)? as usize + 257;
    let ndist = bits.take(5)? as usize + 1;
    let ncode = bits.take(4)? as usize + 4;
    let mut lengths = [0u8; 19];
    for &i in &ORDER[..ncode] {
        lengths[i] = bits.take(3)? as u8;
    }
    let lencode = Code::new(&lengths)?;
    let mut lengths = vec![0u8; nlen + ndist];
    let mut i = 0;
    while i < lengths.len() {
        let (value, repeat) = match lencode.decode(bits)? {
            s @ 0..=15 => (s as u8, 1),
            16 => (*lengths.get(i.checked_sub(1)?)?, 3 + bits.take(2)?),
            17 => (0, 3 + bits.take(3)?),
            18 => (0, 11 + bits.take(7)?),
            _ => return None,
        };
        for _ in 0..repeat {
            *lengths.get_mut(i)? = value;
            i += 1;
        }
    }
    Some((Code::new(&lengths[..nlen])?, Code::new(&lengths[nlen..])?))
}

/// The start of a zlib stream's output: at least `want` bytes, or all of it
/// when it's shorter; none when the input is cut short or malformed.
pub fn inflate(input: &[u8], want: usize) -> Option<Vec<u8>> {
    let (&cmf, &flg) = (input.first()?, input.get(1)?);
    if cmf & 0x0f != 8 || ((u16::from(cmf) << 8) | u16::from(flg)) % 31 != 0 || flg & 0x20 != 0 {
        return None;
    }
    let mut bits = Bits {
        input,
        pos: 2,
        buf: 0,
        count: 0,
    };
    let mut out = Vec::new();
    loop {
        let last = bits.take(1)? == 1;
        match bits.take(2)? {
            0 => {
                (bits.buf, bits.count) = (0, 0);
                let at = bits.pos;
                let len = usize::from(u16::from_le_bytes([*input.get(at)?, *input.get(at + 1)?]));
                out.extend_from_slice(input.get(at + 4..at + 4 + len)?);
                bits.pos = at + 4 + len;
            }
            kind @ (1 | 2) => {
                let (lencode, distcode) = if kind == 1 {
                    fixed()?
                } else {
                    dynamic(&mut bits)?
                };
                loop {
                    let sym = usize::from(lencode.decode(&mut bits)?);
                    if sym == 256 || out.len() >= want {
                        break;
                    }
                    if sym < 256 {
                        out.push(sym as u8);
                        continue;
                    }
                    let s = sym - 257;
                    let len = usize::from(*LEN_BASE.get(s)?)
                        + bits.take(u32::from(LEN_EXTRA[s]))? as usize;
                    let d = usize::from(distcode.decode(&mut bits)?);
                    let dist = usize::from(*DIST_BASE.get(d)?)
                        + bits.take(u32::from(DIST_EXTRA[d]))? as usize;
                    let from = out.len().checked_sub(dist)?;
                    for k in 0..len {
                        out.push(out[from + k]);
                    }
                }
            }
            _ => return None,
        }
        if last || out.len() >= want {
            return Some(out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::inflate;

    fn bytes(hex: &str) -> Vec<u8> {
        (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect()
    }

    const SHORT: &[u8] = b"tree 0123abcd\nparent 0123abcd\n";
    const FIXED: &str = "78da2b294a4d55303034324e4c4a4ee12a482c4acd2b41f0019303092f";

    #[test]
    fn a_stored_block_and_a_fixed_code_block_both_read_back() {
        let stored =
            "7801011e00e1ff747265652030313233616263640a706172656e742030313233616263640a9303092f";
        assert_eq!(inflate(&bytes(stored), usize::MAX).unwrap(), SHORT);
        assert_eq!(inflate(&bytes(FIXED), usize::MAX).unwrap(), SHORT);
    }

    #[test]
    fn a_dynamic_code_block_reads_back_with_its_back_references() {
        let dynamic = "78dad58c571580300c45ad3c0518404d0be96034dd05d493830bbeefa88e909a5f76e8cc23c0f085ad9db1803b6554c1877a6eac6c674425de79438b347c7530be93a087020e9f1a67696d99beec2fd71779f05d82";
        let text = "the quick brown fox jumps over the lazy dog; pack my box with five dozen liquor jugs. ";
        let out = inflate(&bytes(dynamic), usize::MAX).unwrap();
        assert_eq!(out, text.repeat(3).as_bytes());
    }

    #[test]
    fn it_stops_once_it_has_enough_and_refuses_a_cut_or_foreign_stream() {
        let fixed = bytes(FIXED);
        let start = inflate(&fixed[..12], 5).unwrap();
        assert!(start.len() >= 5 && SHORT.starts_with(&start), "{start:?}");
        assert_eq!(inflate(&fixed[..12], usize::MAX), None);
        assert_eq!(inflate(b"PK\x03\x04", 5), None);
    }
}
