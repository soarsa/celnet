//! Agrona-inspired DirectBuffer and MutableDirectBuffer abstractions.
//!
//! Provides zero-allocation, fixed-offset memory buffer access with explicit
//! little-endian byte order, bounds validation, and flyweight support modeled
//! after the Agrona high-performance utility library.
#![deny(missing_docs)]

use crate::SbeError;

/// Immutable direct buffer viewing a contiguous byte slice.
///
/// Models Agrona's `DirectBuffer`, enabling zero-copy field access with
/// guaranteed bounds safety and compiler-vectorized loads.
#[derive(Debug, Clone, Copy)]
pub struct DirectBuffer<'a> {
    slice: &'a [u8],
}

impl<'a> DirectBuffer<'a> {
    /// Wrap an existing immutable byte slice.
    #[inline(always)]
    pub const fn wrap(slice: &'a [u8]) -> Self {
        Self { slice }
    }

    /// Total capacity in bytes.
    #[inline(always)]
    pub const fn capacity(&self) -> usize {
        self.slice.len()
    }

    /// Underlying raw byte slice.
    #[inline(always)]
    pub const fn as_slice(&self) -> &'a [u8] {
        self.slice
    }

    /// Read a single unsigned byte (u8).
    #[inline(always)]
    pub fn get_u8(&self, offset: usize) -> Result<u8, SbeError> {
        if offset >= self.slice.len() {
            return Err(SbeError::BufferTooShort {
                expected: offset + 1,
                actual: self.slice.len(),
            });
        }
        Ok(self.slice[offset])
    }

    /// Read a signed byte (i8).
    #[inline(always)]
    pub fn get_i8(&self, offset: usize) -> Result<i8, SbeError> {
        self.get_u8(offset).map(|b| b as i8)
    }

    /// Read a 16-bit unsigned integer in little-endian.
    #[inline(always)]
    pub fn get_u16_le(&self, offset: usize) -> Result<u16, SbeError> {
        if offset + 2 > self.slice.len() {
            return Err(SbeError::BufferTooShort {
                expected: offset + 2,
                actual: self.slice.len(),
            });
        }
        Ok(u16::from_le_bytes(
            self.slice[offset..offset + 2].try_into().unwrap(),
        ))
    }

    /// Read a 16-bit signed integer in little-endian.
    #[inline(always)]
    pub fn get_i16_le(&self, offset: usize) -> Result<i16, SbeError> {
        self.get_u16_le(offset).map(|v| v as i16)
    }

    /// Read a 32-bit unsigned integer in little-endian.
    #[inline(always)]
    pub fn get_u32_le(&self, offset: usize) -> Result<u32, SbeError> {
        if offset + 4 > self.slice.len() {
            return Err(SbeError::BufferTooShort {
                expected: offset + 4,
                actual: self.slice.len(),
            });
        }
        Ok(u32::from_le_bytes(
            self.slice[offset..offset + 4].try_into().unwrap(),
        ))
    }

    /// Read a 32-bit signed integer in little-endian.
    #[inline(always)]
    pub fn get_i32_le(&self, offset: usize) -> Result<i32, SbeError> {
        self.get_u32_le(offset).map(|v| v as i32)
    }

    /// Read a 64-bit unsigned integer in little-endian.
    #[inline(always)]
    pub fn get_u64_le(&self, offset: usize) -> Result<u64, SbeError> {
        if offset + 8 > self.slice.len() {
            return Err(SbeError::BufferTooShort {
                expected: offset + 8,
                actual: self.slice.len(),
            });
        }
        Ok(u64::from_le_bytes(
            self.slice[offset..offset + 8].try_into().unwrap(),
        ))
    }

    /// Read a 64-bit signed integer in little-endian.
    #[inline(always)]
    pub fn get_i64_le(&self, offset: usize) -> Result<i64, SbeError> {
        if offset + 8 > self.slice.len() {
            return Err(SbeError::BufferTooShort {
                expected: offset + 8,
                actual: self.slice.len(),
            });
        }
        Ok(i64::from_le_bytes(
            self.slice[offset..offset + 8].try_into().unwrap(),
        ))
    }

    /// Read an IEEE 754 32-bit float in little-endian.
    #[inline(always)]
    pub fn get_f32_le(&self, offset: usize) -> Result<f32, SbeError> {
        if offset + 4 > self.slice.len() {
            return Err(SbeError::BufferTooShort {
                expected: offset + 4,
                actual: self.slice.len(),
            });
        }
        Ok(f32::from_le_bytes(
            self.slice[offset..offset + 4].try_into().unwrap(),
        ))
    }

    /// Read an IEEE 754 64-bit double float in little-endian.
    #[inline(always)]
    pub fn get_f64_le(&self, offset: usize) -> Result<f64, SbeError> {
        if offset + 8 > self.slice.len() {
            return Err(SbeError::BufferTooShort {
                expected: offset + 8,
                actual: self.slice.len(),
            });
        }
        Ok(f64::from_le_bytes(
            self.slice[offset..offset + 8].try_into().unwrap(),
        ))
    }

    /// Extract a contiguous sub-slice without copying.
    #[inline(always)]
    pub fn get_bytes(&self, offset: usize, length: usize) -> Result<&'a [u8], SbeError> {
        if offset + length > self.slice.len() {
            return Err(SbeError::BufferTooShort {
                expected: offset + length,
                actual: self.slice.len(),
            });
        }
        Ok(&self.slice[offset..offset + length])
    }

    /// Copy a range of bytes into a destination buffer.
    #[inline(always)]
    pub fn copy_bytes_to(&self, offset: usize, dest: &mut [u8]) -> Result<(), SbeError> {
        let bytes = self.get_bytes(offset, dest.len())?;
        dest.copy_from_slice(bytes);
        Ok(())
    }

    /// Read an ASCII string of given length.
    pub fn get_string_ascii(&self, offset: usize, length: usize) -> Result<&'a str, SbeError> {
        let bytes = self.get_bytes(offset, length)?;
        std::str::from_utf8(bytes).map_err(|_| SbeError::InvalidEnumValue(0))
    }
}

/// Mutable direct buffer viewing a contiguous mutable byte slice.
///
/// Models Agrona's `MutableDirectBuffer`, providing zero-allocation writes
/// with explicit endianness encoding and bounds checks.
#[derive(Debug)]
pub struct MutableDirectBuffer<'a> {
    slice: &'a mut [u8],
}

impl<'a> MutableDirectBuffer<'a> {
    /// Wrap an existing mutable byte slice.
    #[inline(always)]
    pub fn wrap(slice: &'a mut [u8]) -> Self {
        Self { slice }
    }

    /// Total capacity in bytes.
    #[inline(always)]
    pub fn capacity(&self) -> usize {
        self.slice.len()
    }

    /// Borrow as an immutable slice.
    #[inline(always)]
    pub fn as_slice(&self) -> &[u8] {
        self.slice
    }

    /// Borrow as a mutable slice.
    #[inline(always)]
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        self.slice
    }

    /// Convert into an immutable `DirectBuffer`.
    #[inline(always)]
    pub fn into_direct_buffer(self) -> DirectBuffer<'a> {
        DirectBuffer::wrap(self.slice)
    }

    /// Write a single unsigned byte (u8).
    #[inline(always)]
    pub fn put_u8(&mut self, offset: usize, value: u8) -> Result<(), SbeError> {
        if offset >= self.slice.len() {
            return Err(SbeError::BufferTooShort {
                expected: offset + 1,
                actual: self.slice.len(),
            });
        }
        self.slice[offset] = value;
        Ok(())
    }

    /// Write a 16-bit unsigned integer in little-endian.
    #[inline(always)]
    pub fn put_u16_le(&mut self, offset: usize, value: u16) -> Result<(), SbeError> {
        if offset + 2 > self.slice.len() {
            return Err(SbeError::BufferTooShort {
                expected: offset + 2,
                actual: self.slice.len(),
            });
        }
        self.slice[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    /// Write a 32-bit unsigned integer in little-endian.
    #[inline(always)]
    pub fn put_u32_le(&mut self, offset: usize, value: u32) -> Result<(), SbeError> {
        if offset + 4 > self.slice.len() {
            return Err(SbeError::BufferTooShort {
                expected: offset + 4,
                actual: self.slice.len(),
            });
        }
        self.slice[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    /// Write a 64-bit unsigned integer in little-endian.
    #[inline(always)]
    pub fn put_u64_le(&mut self, offset: usize, value: u64) -> Result<(), SbeError> {
        if offset + 8 > self.slice.len() {
            return Err(SbeError::BufferTooShort {
                expected: offset + 8,
                actual: self.slice.len(),
            });
        }
        self.slice[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    /// Write a 64-bit signed integer in little-endian.
    #[inline(always)]
    pub fn put_i64_le(&mut self, offset: usize, value: i64) -> Result<(), SbeError> {
        if offset + 8 > self.slice.len() {
            return Err(SbeError::BufferTooShort {
                expected: offset + 8,
                actual: self.slice.len(),
            });
        }
        self.slice[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    /// Write an IEEE 754 64-bit double float in little-endian.
    #[inline(always)]
    pub fn put_f64_le(&mut self, offset: usize, value: f64) -> Result<(), SbeError> {
        if offset + 8 > self.slice.len() {
            return Err(SbeError::BufferTooShort {
                expected: offset + 8,
                actual: self.slice.len(),
            });
        }
        self.slice[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    /// Write arbitrary bytes from a source slice.
    #[inline(always)]
    pub fn put_bytes(&mut self, offset: usize, src: &[u8]) -> Result<(), SbeError> {
        if offset + src.len() > self.slice.len() {
            return Err(SbeError::BufferTooShort {
                expected: offset + src.len(),
                actual: self.slice.len(),
            });
        }
        self.slice[offset..offset + src.len()].copy_from_slice(src);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_direct_buffer_primitives() {
        let mut raw = [0u8; 64];
        let mut mdb = MutableDirectBuffer::wrap(&mut raw);

        mdb.put_u8(0, 42).unwrap();
        mdb.put_u16_le(1, 1024).unwrap();
        mdb.put_u32_le(4, 999_999).unwrap();
        mdb.put_u64_le(8, 1_234_567_890_123_456).unwrap();
        mdb.put_f64_le(16, 3.141592653589793).unwrap();
        mdb.put_bytes(24, b"CELNET_AGRONA").unwrap();

        let db = DirectBuffer::wrap(&raw);
        assert_eq!(db.get_u8(0).unwrap(), 42);
        assert_eq!(db.get_u16_le(1).unwrap(), 1024);
        assert_eq!(db.get_u32_le(4).unwrap(), 999_999);
        assert_eq!(db.get_u64_le(8).unwrap(), 1_234_567_890_123_456);
        assert_eq!(db.get_f64_le(16).unwrap(), 3.141592653589793);
        assert_eq!(db.get_bytes(24, 13).unwrap(), b"CELNET_AGRONA");
        assert_eq!(db.get_string_ascii(24, 13).unwrap(), "CELNET_AGRONA");
    }

    #[test]
    fn test_bounds_error() {
        let raw = [0u8; 10];
        let db = DirectBuffer::wrap(&raw);
        assert!(db.get_u64_le(8).is_err());
    }
}
