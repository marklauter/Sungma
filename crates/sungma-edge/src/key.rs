//! Keys: a partition and a sort key, each built from parts that encode so
//! they sort in the order the parts do. A number is written big-endian at
//! its full width, and text is its bytes and a zero byte, which sorts below
//! every other byte, so a shorter text sorts before a longer one it starts.

use std::fmt;

/// A key part that encodes so encoded parts sort as the values do.
pub trait Part {
    fn encode(&self, key: &mut Vec<u8>);
}

impl Part for u8 {
    fn encode(&self, key: &mut Vec<u8>) {
        key.push(*self);
    }
}

impl Part for u32 {
    fn encode(&self, key: &mut Vec<u8>) {
        key.extend_from_slice(&self.to_be_bytes());
    }
}

impl Part for u64 {
    fn encode(&self, key: &mut Vec<u8>) {
        key.extend_from_slice(&self.to_be_bytes());
    }
}

/// Text holds no zero byte; a zero byte inside text would end it early
/// when the key is read back.
impl Part for str {
    fn encode(&self, key: &mut Vec<u8>) {
        key.extend_from_slice(self.as_bytes());
        key.push(0);
    }
}

impl<P: Part + ?Sized> Part for &P {
    fn encode(&self, key: &mut Vec<u8>) {
        P::encode(self, key);
    }
}

macro_rules! tuple {
    ($($part:ident),+) => {
        impl<$($part: Part),+> Part for ($($part,)+) {
            #[allow(non_snake_case)]
            fn encode(&self, key: &mut Vec<u8>) {
                let ($($part,)+) = self;
                $($part.encode(key);)+
            }
        }
    };
}

tuple!(A);
tuple!(A, B);
tuple!(A, B, C);
tuple!(A, B, C, D);
tuple!(A, B, C, D, E);

/// The bytes `part` encodes to.
pub fn encode(part: impl Part) -> Vec<u8> {
    let mut key = Vec::new();
    part.encode(&mut key);
    key
}

/// An item's address: the partition it's in, and its sort key there.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Key {
    pub partition: Vec<u8>,
    pub sort: Vec<u8>,
}

impl Key {
    pub fn new(partition: impl Part, sort: impl Part) -> Self {
        Self {
            partition: encode(partition),
            sort: encode(sort),
        }
    }
}

/// Hex, partition and sort key apart.
impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        write!(f, "Key({}/{})", hex(&self.partition), hex(&self.sort))
    }
}

/// Reads a key back part by part. Each read gives `None` when the bytes
/// left don't hold the part.
pub struct Parts<'a>(&'a [u8]);

impl<'a> Parts<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self(bytes)
    }

    pub fn byte(&mut self) -> Option<u8> {
        let (&first, rest) = self.0.split_first()?;
        self.0 = rest;
        Some(first)
    }

    pub fn u32(&mut self) -> Option<u32> {
        self.take().map(u32::from_be_bytes)
    }

    pub fn u64(&mut self) -> Option<u64> {
        self.take().map(u64::from_be_bytes)
    }

    pub fn text(&mut self) -> Option<&'a str> {
        let end = self.0.iter().position(|&byte| byte == 0)?;
        let text = std::str::from_utf8(&self.0[..end]).ok()?;
        self.0 = &self.0[end + 1..];
        Some(text)
    }

    /// `Some(value)` only when every byte was read.
    pub fn end<T>(&self, value: T) -> Option<T> {
        self.0.is_empty().then_some(value)
    }

    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let (bytes, rest) = self.0.split_first_chunk::<N>()?;
        self.0 = rest;
        Some(*bytes)
    }
}
