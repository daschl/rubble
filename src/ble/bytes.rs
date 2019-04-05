//! Utilities for decoding from and encoding into bytes.
//!
//! The most important parts offered by this module are [`ToBytes`] and [`FromBytes`],
//! general-purpose traits for zero-allocation (de)serialization from/to bytes. These are used all
//! over the place in Rubble, along with [`ByteWriter`]. In addition to those, this module also
//! defines helpful extension traits on `&[T]` and `&mut [T]`, defined on [`SliceExt`] and
//! [`MutSliceExt`], and on `&[u8]`, defined on [`BytesExt`].
//!
//! [`ToBytes`]: trait.ToBytes.html
//! [`FromBytes`]: trait.FromBytes.html
//! [`ByteWriter`]: struct.ByteWriter.html
//! [`SliceExt`]: trait.SliceExt.html
//! [`MutSliceExt`]: trait.MutSliceExt.html
//! [`BytesExt`]: trait.BytesExt.html

use {
    crate::ble::Error,
    byteorder::ByteOrder,
    core::{fmt, iter, mem},
};

pub use byteorder::LittleEndian;

/// Reference to a `T`, or to a byte slice that can be decoded as a `T`.
///
/// This type can be used in structures when storing a `T` directly is not desirable. For example,
/// `T` might actually be a slice `[U]` and thus has a non-static size, or `T`s size might simply be
/// too large (eg. it could be inside a rarely-encountered variant or would blow up the total size
/// of the containing enum).
///
/// The size of this type is currently 2 `usize`s plus a discriminant byte, but could potentially be
/// (unsafely) reduced further, should that be required.
pub struct BytesOr<'a, T: ?Sized>(Inner<'a, T>);

impl<'a, T: ?Sized> From<&'a T> for BytesOr<'a, T> {
    fn from(r: &'a T) -> Self {
        BytesOr(Inner::Or(r))
    }
}

enum Inner<'a, T: ?Sized> {
    Bytes(&'a [u8]),
    Or(&'a T),
}

impl<'a, T: ?Sized> Clone for Inner<'a, T> {
    fn clone(&self) -> Self {
        match self {
            Inner::Bytes(b) => Inner::Bytes(b),
            Inner::Or(t) => Inner::Or(t),
        }
    }
}

impl<'a, T: ?Sized> Clone for BytesOr<'a, T> {
    fn clone(&self) -> Self {
        BytesOr(self.0)
    }
}

impl<'a, T: ?Sized> Copy for BytesOr<'a, T> {}
impl<'a, T: ?Sized> Copy for Inner<'a, T> {}

impl<'a, T: fmt::Debug + FromBytes<'a> + Copy> fmt::Debug for BytesOr<'a, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.read().fmt(f)
    }
}

impl<'a, T: fmt::Debug + FromBytes<'a> + Copy> fmt::Debug for BytesOr<'a, [T]> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<'a, T: ?Sized> BytesOr<'a, T> {
    /// Creates a `BytesOr` that holds on to a `T` via reference.
    ///
    /// For creating a `BytesOr` that references a byte slice, the [`FromBytes`] impl(s) can be
    /// used.
    ///
    /// [`FromBytes`]: trait.FromBytes.html
    pub fn from_ref(value: &'a T) -> Self {
        BytesOr(Inner::Or(value))
    }
}

/// Creates a `BytesOr` that stores bytes that can be decoded to a `T`.
///
/// This will check that `bytes` can indeed be decoded as a `T` using its [`FromBytes`]
/// implementation, and returns an error if not. An error will also be returned if `bytes` contains
/// more data than needed for a `T`.
///
/// [`FromBytes`]: trait.FromBytes.html
// FIXME this is the only `FromBytes` impl that does something like this, maybe it shouln't?
impl<'a, T: FromBytes<'a>> FromBytes<'a> for BytesOr<'a, T> {
    fn from_bytes(bytes: &mut &'a [u8]) -> Result<Self, Error> {
        {
            let bytes = &mut &**bytes;
            T::from_bytes(bytes)?;
            if !bytes.is_empty() {
                return Err(Error::IncompleteParse);
            }
        }
        Ok(BytesOr(Inner::Bytes(bytes)))
    }
}

/// Creates a `BytesOr` that stores bytes that can be decoded to a sequence of `T`s.
///
/// This will check that `bytes` can indeed be decoded as a sequence of `T`s, and returns an error
/// if not.
impl<'a, T: FromBytes<'a>> FromBytes<'a> for BytesOr<'a, [T]> {
    fn from_bytes(bytes: &mut &'a [u8]) -> Result<Self, Error> {
        {
            let bytes = &mut &**bytes;
            while !bytes.is_empty() {
                T::from_bytes(bytes)?;
            }
        }
        Ok(BytesOr(Inner::Bytes(bytes)))
    }
}

impl<'a, T: ToBytes> ToBytes for BytesOr<'a, T> {
    fn to_bytes(&self, buffer: &mut ByteWriter) -> Result<(), Error> {
        match self.0 {
            Inner::Bytes(b) => buffer.write_slice(b),
            Inner::Or(t) => t.to_bytes(buffer),
        }
    }
}

impl<'a, T: ToBytes> ToBytes for BytesOr<'a, [T]> {
    fn to_bytes(&self, buffer: &mut ByteWriter) -> Result<(), Error> {
        match self.0 {
            Inner::Bytes(b) => buffer.write_slice(b),
            Inner::Or(ts) => {
                for t in ts {
                    t.to_bytes(buffer)?;
                }
                Ok(())
            }
        }
    }
}

impl<'a, T: Copy + FromBytes<'a>> BytesOr<'a, T> {
    /// Reads the `T`, possibly by parsing the stored bytes.
    ///
    /// If `self` already stores a reference to a `T`, the `T` will just be copied out. If `self`
    /// stores a byte slice, the `T` will be parsed using its [`FromBytes`] implementation.
    ///
    /// [`FromBytes`]: trait.FromBytes.html
    pub fn read(&self) -> T {
        match self.0 {
            Inner::Bytes(mut b) => {
                let t = T::from_bytes(&mut b).unwrap();
                assert!(b.is_empty());
                t
            }
            Inner::Or(t) => *t,
        }
    }
}

impl<'a, T: Copy + FromBytes<'a>> BytesOr<'a, T> {
    /// Returns an iterator over all `T`s stored in `self` (which is just one `T` in this case).
    ///
    /// This method exists to mirror its twin implemented for `BytesOr<'a, [T]>`.
    pub fn iter(&self) -> impl Iterator<Item = T> + 'a {
        iter::once(self.read())
    }
}

impl<'a, T: Copy + FromBytes<'a>> BytesOr<'a, [T]> {
    /// Returns an iterator over all `T`s stored in `self`.
    ///
    /// The iterator will copy or decode `T`s out of `self`.
    pub fn iter(&self) -> impl Iterator<Item = T> + 'a {
        IterBytesOr { inner: *self }
    }
}

/// An iterator over values stored in a `BytesOr`.
struct IterBytesOr<'a, T> {
    inner: BytesOr<'a, [T]>,
}

impl<'a, T: Copy + FromBytes<'a>> Iterator for IterBytesOr<'a, T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.inner.0 {
            Inner::Bytes(b) => {
                if b.is_empty() {
                    None
                } else {
                    Some(T::from_bytes(b).unwrap())
                }
            }
            Inner::Or(slice) => slice.read_first().ok(),
        }
    }
}

/// Wrapper around a byte slice that can be used to encode data into bytes.
///
/// All `write_*` methods on this type will return `Error::Eof` when the underlying buffer slice is
/// full.
pub struct ByteWriter<'a>(&'a mut [u8]);

impl<'a> ByteWriter<'a> {
    /// Creates a writer that will write to `buf`.
    pub fn new(buf: &'a mut [u8]) -> Self {
        ByteWriter(buf)
    }

    /// Consumes `self` and returns the part of the contained buffer that has not yet been written
    /// to.
    pub fn into_rest(self) -> &'a mut [u8] {
        self.0
    }

    /// Returns the raw buffer this `ByteWriter` would write to.
    ///
    /// Combined with `skip`, this method allows advanced operations on the underlying byte buffer.
    pub fn rest(&mut self) -> &mut [u8] {
        self.0
    }

    /// Skips the given number of bytes in the output data without writing anything there.
    ///
    /// This is a potentially dangerous operation that should only be used when necessary (eg. when
    /// the skipped data will be filled in by other code). If the skipped bytes are *not* written,
    /// they will probably contain garbage data from an earlier use of the underlying buffer.
    pub fn skip(&mut self, bytes: usize) -> Result<(), Error> {
        if self.space_left() < bytes {
            Err(Error::Eof)
        } else {
            let this = mem::replace(&mut self.0, &mut []);
            self.0 = &mut this[bytes..];
            Ok(())
        }
    }

    /// Creates and returns another `ByteWriter` that can write to the next `len` Bytes in the
    /// buffer.
    ///
    /// `self` will be modified to point after the split-off bytes.
    ///
    /// Note that if the created `ByteWriter` is not used, the bytes will contain whatever contents
    /// they had before creating `self` (ie. most likely garbage data left over from earlier use).
    /// If you are really sure you want that, `skip` is a more explicit way of accomplishing that.
    #[must_use]
    pub fn split_off(&mut self, len: usize) -> Result<Self, Error> {
        if self.space_left() < len {
            Err(Error::Eof)
        } else {
            let this = mem::replace(&mut self.0, &mut []);
            let (head, tail) = this.split_at_mut(len);
            self.0 = tail;
            Ok(ByteWriter::new(head))
        }
    }

    /// Returns the number of bytes that can be written to `self` until it is full.
    pub fn space_left(&self) -> usize {
        self.0.len()
    }

    /// Writes a single byte to `self`.
    ///
    /// Returns `Error::Eof` when no space is left.
    pub fn write_byte<'b>(&'b mut self, byte: u8) -> Result<(), Error>
    where
        'a: 'b,
    {
        let first = self.split_next_mut().ok_or(Error::Eof)?;
        *first = byte;
        Ok(())
    }

    /// Writes all bytes from `other` to `self`.
    ///
    /// Returns `Error::Eof` when `self` does not have enough space left to fit `other`. In that
    /// case, `self` will not be modified.
    pub fn write_slice<'b>(&'b mut self, other: &[u8]) -> Result<(), Error>
    where
        'a: 'b,
    {
        if self.space_left() < other.len() {
            Err(Error::Eof)
        } else {
            self.0[..other.len()].copy_from_slice(other);
            let this = mem::replace(&mut self.0, &mut []);
            self.0 = &mut this[other.len()..];
            Ok(())
        }
    }

    /// Writes a `u16` to `self`, using byte order `B`.
    ///
    /// If `self` does not have enough space left, an error will be returned and no bytes will be
    /// written to `self`.
    pub fn write_u16<'b, B: ByteOrder>(&'b mut self, value: u16) -> Result<(), Error>
    where
        'a: 'b,
    {
        let mut bytes = [0; 2];
        B::write_u16(&mut bytes, value);
        self.write_slice(&bytes)
    }

    /// Writes a `u32` to `self`, using byte order `B`.
    ///
    /// If `self` does not have enough space left, an error will be returned and no bytes will be
    /// written to `self`.
    pub fn write_u32<'b, B: ByteOrder>(&'b mut self, value: u32) -> Result<(), Error>
    where
        'a: 'b,
    {
        let mut bytes = [0; 4];
        B::write_u32(&mut bytes, value);
        self.write_slice(&bytes)
    }

    /// Writes a `u64` to `self`, using byte order `B`.
    ///
    /// If `self` does not have enough space left, an error will be returned and no bytes will be
    /// written to `self`.
    pub fn write_u64<'b, B: ByteOrder>(&'b mut self, value: u64) -> Result<(), Error>
    where
        'a: 'b,
    {
        let mut bytes = [0; 8];
        B::write_u64(&mut bytes, value);
        self.write_slice(&bytes)
    }

    /// Splits off the next byte in the buffer.
    ///
    /// The writer will be advanced to point to the rest of the underlying buffer.
    ///
    /// This allows filling in the value of the byte later, after writing more data.
    ///
    /// For a similar, but more flexible operation, see [`split_off`].
    ///
    /// [`split_off`]: #method.split_off
    pub fn split_next_mut<'b>(&'b mut self) -> Option<&'a mut u8>
    where
        'a: 'b,
    {
        let this = mem::replace(&mut self.0, &mut []);
        // Slight contortion to please the borrow checker:
        if this.is_empty() {
            self.0 = this;
            None
        } else {
            let (first, rest) = this.split_first_mut().unwrap();
            self.0 = rest;
            Some(first)
        }
    }
}

/// Trait for encoding a value into a byte buffer.
pub trait ToBytes {
    /// Converts `self` to bytes and writes them into `buffer`, advancing `buffer` to point past the
    /// encoded value.
    ///
    /// If `buffer` does not contain enough space, an error will be returned and the state of the
    /// buffer is unspecified (eg. `self` may be partially written into `buffer`).
    fn to_bytes(&self, writer: &mut ByteWriter) -> Result<(), Error>;
}

/// Trait for decoding values from a byte slice.
pub trait FromBytes<'a>: Sized {
    /// Decode a `Self` from a byte slice, advancing `bytes` to point past the data that was read.
    ///
    /// If `bytes` contains data not valid for the target type, or contains an insufficient number
    /// of bytes, an error will be returned and the state of `bytes` is unspecified (it can point to
    /// arbitrary data).
    fn from_bytes(bytes: &mut &'a [u8]) -> Result<Self, Error>;
}

impl ToBytes for [u8] {
    fn to_bytes(&self, writer: &mut ByteWriter) -> Result<(), Error> {
        writer.write_slice(self)
    }
}

impl<'a> ToBytes for &'a [u8] {
    fn to_bytes(&self, writer: &mut ByteWriter) -> Result<(), Error> {
        writer.write_slice(*self)
    }
}

impl<'a> FromBytes<'a> for &'a [u8] {
    fn from_bytes(bytes: &mut &'a [u8]) -> Result<Self, Error> {
        Ok(mem::replace(bytes, &[]))
    }
}

impl<'a> FromBytes<'a> for u8 {
    fn from_bytes(bytes: &mut &'a [u8]) -> Result<Self, Error> {
        bytes.read_u8()
    }
}

/// Extensions on `&'a [u8]` that expose byteorder methods.
pub trait BytesExt<'a> {
    fn read_u8(&mut self) -> Result<u8, Error>;
    fn read_u16<B: ByteOrder>(&mut self) -> Result<u16, Error>;
    fn read_u32<B: ByteOrder>(&mut self) -> Result<u32, Error>;
    fn read_u64<B: ByteOrder>(&mut self) -> Result<u64, Error>;
}

impl<'a> BytesExt<'a> for &'a [u8] {
    fn read_u8(&mut self) -> Result<u8, Error> {
        Ok(self.read_array::<[u8; 1]>()?[0])
    }

    fn read_u16<B: ByteOrder>(&mut self) -> Result<u16, Error> {
        let arr = self.read_array::<[u8; 2]>()?;
        Ok(B::read_u16(&arr))
    }

    fn read_u32<B: ByteOrder>(&mut self) -> Result<u32, Error> {
        let arr = self.read_array::<[u8; 4]>()?;
        Ok(B::read_u32(&arr))
    }

    fn read_u64<B: ByteOrder>(&mut self) -> Result<u64, Error> {
        let arr = self.read_array::<[u8; 8]>()?;
        Ok(B::read_u64(&arr))
    }
}

/// Extensions on `&'a [T]`.
pub trait SliceExt<'a, T: Copy> {
    /// Returns a copy of the first element in the slice `self` and advances `self` to point past
    /// the element.
    fn read_first(&mut self) -> Result<T, Error>;

    /// Reads an array-like type `S` out of `self`.
    ///
    /// `self` will be updated to point past the read data.
    ///
    /// If `self` doesn't contain enough elements to fill an `S`, returns `Error::Eof` without
    /// changing `self`.
    fn read_array<S>(&mut self) -> Result<S, Error>
    where
        S: Default + AsMut<[T]>;

    /// Reads a slice of `len` items from `self`.
    ///
    /// `self` will be updated to point past the extracted elements.
    ///
    /// If `self` does not contains `len` elements, `Error::Eof` will be returned and `self` will
    /// not be modified.
    fn read_slice(&mut self, len: usize) -> Result<&'a [T], Error>;
}

impl<'a, T: Copy> SliceExt<'a, T> for &'a [T] {
    fn read_first(&mut self) -> Result<T, Error> {
        let (first, rest) = self.split_first().ok_or(Error::Eof)?;
        *self = rest;
        Ok(*first)
    }

    fn read_array<S>(&mut self) -> Result<S, Error>
    where
        S: Default + AsMut<[T]>,
    {
        let mut buf = S::default();
        let slice = buf.as_mut();
        if self.len() < slice.len() {
            return Err(Error::Eof);
        }

        slice.copy_from_slice(&self[..slice.len()]);
        *self = &self[slice.len()..];
        Ok(buf)
    }

    fn read_slice(&mut self, len: usize) -> Result<&'a [T], Error> {
        if self.len() < len {
            Err(Error::Eof)
        } else {
            let slice = &self[..len];
            *self = &self[len..];
            Ok(slice)
        }
    }
}

/// Extensions on `&'a mut [u8]`.
pub trait MutSliceExt<'a> {
    /// Writes a byte to the beginning of `self` and updates `self` to point behind the written
    /// byte.
    ///
    /// If `self` is empty, returns an error.
    fn write_byte<'b>(&'b mut self, byte: u8) -> Result<(), Error>
    where
        'a: 'b;

    /// Copies all elements from `other` into `self` and advances `self` to point behind the copied
    /// elements.
    ///
    /// If `self` is empty, returns an error.
    fn write_slice<'b>(&'b mut self, other: &[u8]) -> Result<(), Error>
    where
        'a: 'b;
}

impl<'a> MutSliceExt<'a> for &'a mut [u8] {
    fn write_byte<'b>(&'b mut self, byte: u8) -> Result<(), Error>
    where
        'a: 'b,
    {
        // The `mem::replace` is needed to work around a complex borrowing restriction:
        // If we had `'b: 'a` instead of `'a: 'b`, a call to `write_byte` would result in the
        // "infinite self-borrow" problem, which makes the method useless. The `'a: 'b` means that
        // we have a `&'b mut &'a mut [u8]`, and we could only get a shortened `&'b mut [u8]` out of
        // that (for soundness reasons - the same thing that makes invariance necessary).
        // By using `mem::replace` we can safely get a `&'a mut [u8]` out instead (replacing what's
        // behind the reference with a `&'static mut []`).
        match mem::replace(self, &mut []).split_first_mut() {
            Some((first, rest)) => {
                *first = byte;
                *self = rest;
                Ok(())
            }
            None => Err(Error::Eof),
        }
    }

    fn write_slice<'b>(&'b mut self, other: &[u8]) -> Result<(), Error>
    where
        'a: 'b,
    {
        if self.len() < other.len() {
            Err(Error::Eof)
        } else {
            self[..other.len()].copy_from_slice(other);
            let this = mem::replace(self, &mut []);
            *self = &mut this[other.len()..];
            Ok(())
        }
    }
}
