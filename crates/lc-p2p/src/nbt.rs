//! Minimal lossless NBT (uncompressed, big-endian, Java edition) reader and
//! writer, enough to edit `servers.dat` without disturbing anything else.
//!
//! Strings are kept as raw "modified UTF-8" bytes so entries we don't touch
//! round-trip byte for byte; floats are kept as bit patterns.

use std::fmt;

/// Nesting limit: real files are shallow; this only guards the stack.
const MAX_DEPTH: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Tag {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    /// Bit pattern of an `f32`.
    Float(u32),
    /// Bit pattern of an `f64`.
    Double(u64),
    ByteArray(Vec<u8>),
    /// Raw modified UTF-8.
    String(Vec<u8>),
    /// Element type id + elements (all of that type).
    List(u8, Vec<Tag>),
    /// Entries in file order.
    Compound(Vec<(Vec<u8>, Tag)>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NbtError(&'static str);

impl fmt::Display for NbtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid NBT: {}", self.0)
    }
}

impl std::error::Error for NbtError {}

impl NbtError {
    pub(crate) fn new(what: &'static str) -> Self {
        Self(what)
    }
}

impl Tag {
    pub(crate) fn id(&self) -> u8 {
        match self {
            Tag::Byte(_) => 1,
            Tag::Short(_) => 2,
            Tag::Int(_) => 3,
            Tag::Long(_) => 4,
            Tag::Float(_) => 5,
            Tag::Double(_) => 6,
            Tag::ByteArray(_) => 7,
            Tag::String(_) => 8,
            Tag::List(..) => 9,
            Tag::Compound(_) => 10,
            Tag::IntArray(_) => 11,
            Tag::LongArray(_) => 12,
        }
    }

    pub(crate) fn string(s: &str) -> Tag {
        Tag::String(to_mutf8(s))
    }

    /// Value of a string entry of a compound.
    pub(crate) fn get_str(&self, key: &str) -> Option<&[u8]> {
        match self.get(key)? {
            Tag::String(s) => Some(s),
            _ => None,
        }
    }

    pub(crate) fn get(&self, key: &str) -> Option<&Tag> {
        let Tag::Compound(entries) = self else {
            return None;
        };
        let key = to_mutf8(key);
        entries.iter().find(|(k, _)| *k == key).map(|(_, v)| v)
    }
}

/// Java's modified UTF-8: NUL as `C0 80`, supplementary characters as
/// two 3-byte surrogates.
pub(crate) fn to_mutf8(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\0' => out.extend_from_slice(&[0xC0, 0x80]),
            c if (c as u32) < 0x10000 => {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
            c => {
                let mut units = [0u16; 2];
                for u in c.encode_utf16(&mut units).iter() {
                    let u = u32::from(*u);
                    out.push(0xE0 | ((u >> 12) & 0x0F) as u8);
                    out.push(0x80 | ((u >> 6) & 0x3F) as u8);
                    out.push(0x80 | (u & 0x3F) as u8);
                }
            }
        }
    }
    out
}

/// Root compound: (name, tag).
pub(crate) fn read(data: &[u8]) -> Result<(Vec<u8>, Tag), NbtError> {
    let mut r = Reader { data, pos: 0 };
    let id = r.u8()?;
    if id != 10 {
        return Err(NbtError("root is not a compound"));
    }
    let name = r.string()?;
    let tag = r.payload(id, 0)?;
    if r.pos != data.len() {
        return Err(NbtError("trailing bytes"));
    }
    Ok((name, tag))
}

pub(crate) fn write(name: &[u8], root: &Tag) -> Result<Vec<u8>, NbtError> {
    let mut out = Vec::new();
    out.push(root.id());
    put_string(&mut out, name)?;
    put_payload(&mut out, root)?;
    Ok(out)
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], NbtError> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|e| *e <= self.data.len())
            .ok_or(NbtError("unexpected end"))?;
        let s = &self.data[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn arr<const N: usize>(&mut self) -> Result<[u8; N], NbtError> {
        let mut a = [0u8; N];
        a.copy_from_slice(self.take(N)?);
        Ok(a)
    }

    fn u8(&mut self) -> Result<u8, NbtError> {
        Ok(self.arr::<1>()?[0])
    }

    fn i32(&mut self) -> Result<i32, NbtError> {
        Ok(i32::from_be_bytes(self.arr()?))
    }

    fn i64(&mut self) -> Result<i64, NbtError> {
        Ok(i64::from_be_bytes(self.arr()?))
    }

    fn string(&mut self) -> Result<Vec<u8>, NbtError> {
        let len = u16::from_be_bytes(self.arr()?);
        Ok(self.take(usize::from(len))?.to_vec())
    }

    /// Array / list length; never trusts it beyond the bytes left.
    fn len(&mut self, elem: usize) -> Result<usize, NbtError> {
        let n = usize::try_from(self.i32()?).map_err(|_| NbtError("negative length"))?;
        if n.saturating_mul(elem) > self.data.len() - self.pos {
            return Err(NbtError("length past end"));
        }
        Ok(n)
    }

    fn payload(&mut self, id: u8, depth: usize) -> Result<Tag, NbtError> {
        if depth > MAX_DEPTH {
            return Err(NbtError("too deep"));
        }
        Ok(match id {
            1 => Tag::Byte(i8::from_be_bytes(self.arr()?)),
            2 => Tag::Short(i16::from_be_bytes(self.arr()?)),
            3 => Tag::Int(self.i32()?),
            4 => Tag::Long(self.i64()?),
            5 => Tag::Float(u32::from_be_bytes(self.arr()?)),
            6 => Tag::Double(u64::from_be_bytes(self.arr()?)),
            7 => {
                let n = self.len(1)?;
                Tag::ByteArray(self.take(n)?.to_vec())
            }
            8 => Tag::String(self.string()?),
            9 => {
                let elem = self.u8()?;
                // Each element is at least one byte (except End, which has none).
                let n = self.len(usize::from(elem != 0))?;
                if elem == 0 && n != 0 {
                    return Err(NbtError("list of End"));
                }
                let mut items = Vec::with_capacity(n);
                for _ in 0..n {
                    items.push(self.payload(elem, depth + 1)?);
                }
                Tag::List(elem, items)
            }
            10 => {
                let mut entries = Vec::new();
                loop {
                    let id = self.u8()?;
                    if id == 0 {
                        break;
                    }
                    let name = self.string()?;
                    entries.push((name, self.payload(id, depth + 1)?));
                }
                Tag::Compound(entries)
            }
            11 => {
                let n = self.len(4)?;
                Tag::IntArray((0..n).map(|_| self.i32()).collect::<Result<_, _>>()?)
            }
            12 => {
                let n = self.len(8)?;
                Tag::LongArray((0..n).map(|_| self.i64()).collect::<Result<_, _>>()?)
            }
            _ => return Err(NbtError("unknown tag id")),
        })
    }
}

fn put_string(out: &mut Vec<u8>, s: &[u8]) -> Result<(), NbtError> {
    let len = u16::try_from(s.len()).map_err(|_| NbtError("string too long"))?;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(s);
    Ok(())
}

fn put_len(out: &mut Vec<u8>, n: usize) -> Result<(), NbtError> {
    let n = i32::try_from(n).map_err(|_| NbtError("array too long"))?;
    out.extend_from_slice(&n.to_be_bytes());
    Ok(())
}

fn put_payload(out: &mut Vec<u8>, tag: &Tag) -> Result<(), NbtError> {
    match tag {
        Tag::Byte(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Short(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Int(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Long(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Float(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Double(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::ByteArray(v) => {
            put_len(out, v.len())?;
            out.extend_from_slice(v);
        }
        Tag::String(s) => put_string(out, s)?,
        Tag::List(elem, items) => {
            if items.iter().any(|t| t.id() != *elem) {
                return Err(NbtError("mixed list"));
            }
            out.push(*elem);
            put_len(out, items.len())?;
            for t in items {
                put_payload(out, t)?;
            }
        }
        Tag::Compound(entries) => {
            for (name, t) in entries {
                out.push(t.id());
                put_string(out, name)?;
                put_payload(out, t)?;
            }
            out.push(0);
        }
        Tag::IntArray(v) => {
            put_len(out, v.len())?;
            for x in v {
                out.extend_from_slice(&x.to_be_bytes());
            }
        }
        Tag::LongArray(v) => {
            put_len(out, v.len())?;
            for x in v {
                out.extend_from_slice(&x.to_be_bytes());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every tag type once, nested.
    fn all_types() -> Tag {
        Tag::Compound(vec![
            (b"b".to_vec(), Tag::Byte(-1)),
            (b"s".to_vec(), Tag::Short(-300)),
            (b"i".to_vec(), Tag::Int(70_000)),
            (b"l".to_vec(), Tag::Long(-5_000_000_000)),
            (b"f".to_vec(), Tag::Float(1.5f32.to_bits())),
            (b"d".to_vec(), Tag::Double(f64::NAN.to_bits())),
            (b"ba".to_vec(), Tag::ByteArray(vec![0, 255, 7])),
            (b"str".to_vec(), Tag::string("Привет\0😀")),
            (b"empty".to_vec(), Tag::List(0, vec![])),
            (
                b"list".to_vec(),
                Tag::List(10, vec![Tag::Compound(vec![]), Tag::Compound(vec![])]),
            ),
            (b"ia".to_vec(), Tag::IntArray(vec![1, -2])),
            (b"la".to_vec(), Tag::LongArray(vec![i64::MIN])),
        ])
    }

    #[test]
    fn round_trips_every_type() {
        let root = all_types();
        let bytes = write(b"", &root).unwrap();
        let (name, back) = read(&bytes).unwrap();
        assert!(name.is_empty());
        assert_eq!(back, root);
        assert_eq!(write(&name, &back).unwrap(), bytes);
    }

    #[test]
    fn modified_utf8() {
        assert_eq!(to_mutf8("a\0"), vec![b'a', 0xC0, 0x80]);
        assert_eq!(to_mutf8("—"), "—".as_bytes());
        // U+1F600 -> D83D DE00 as two 3-byte sequences.
        assert_eq!(to_mutf8("😀"), vec![0xED, 0xA0, 0xBD, 0xED, 0xB8, 0x80]);
    }

    #[test]
    fn rejects_garbage() {
        assert!(read(&[]).is_err());
        assert!(read(&[8, 0, 0]).is_err());
        // Compound without End.
        assert!(read(&[10, 0, 0, 1, 0, 1, b'x', 5]).is_err());
        // Huge list length.
        assert!(read(&[10, 0, 0, 9, 0, 1, b'x', 10, 0x7F, 0xFF, 0xFF, 0xFF]).is_err());
        // Negative array length.
        assert!(read(&[10, 0, 0, 7, 0, 1, b'x', 0xFF, 0xFF, 0xFF, 0xFF, 0]).is_err());
        // Trailing bytes.
        assert!(read(&[10, 0, 0, 0, 0]).is_err());
        // Deep nesting doesn't overflow the stack.
        let mut deep = vec![10, 0, 0];
        for _ in 0..2000 {
            deep.extend_from_slice(&[10, 0, 0]);
        }
        assert!(read(&deep).is_err());
    }
}
