// Copyright 2016 Masaki Hara
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use std::io;

#[cfg(feature = "bigint")]
use num::bigint::{BigUint, BigInt};

use super::*;

/// Constructs DER-encoded data as `Vec<u8>`.
///
/// This function uses the loan pattern: `callback` is called back with
/// a [`DERWriter`][derwriter], to which the ASN.1 value is written.
///
/// [derwriter]: struct.DERWriter.html
///
/// # Examples
///
/// ```
/// use yasna;
/// let der = yasna::construct_der(|writer| {
///     writer.write_sequence(|writer| {
///         try!(writer.next().write_i64(10));
///         try!(writer.next().write_bool(true));
///         return Ok(());
///     })
/// }).unwrap();
/// assert_eq!(der, vec![48, 6, 2, 1, 10, 1, 1, 255]);
/// ```
///
/// # Errors
///
/// This function just carries errors generated by `callback`.
///
pub fn construct_der<F>(callback: F) -> io::Result<Vec<u8>>
        where F: FnOnce(DERWriter) -> io::Result<()> {
    let mut buf = Vec::new();
    {
        let mut writer = DERWriterSeq {
            buf: &mut buf,
        };
        try!(callback(writer.next()));
    }
    return Ok(buf);
}

/// Constructs DER-encoded sequence of data as `Vec<u8>`.
///
/// This is similar to [`construct_der`][construct_der], but this function
/// accepts more than one ASN.1 values.
///
/// [construct_der]: fn.construct_der.html
///
/// This function uses the loan pattern: `callback` is called back with
/// a [`DERWriterSeq`][derwriterseq], to which the ASN.1 values are written.
///
/// [derwriterseq]: struct.DERWriterSeq.html
///
/// # Examples
///
/// ```
/// use yasna;
/// let der = yasna::construct_der_seq(|writer| {
///     try!(writer.next().write_i64(10));
///     try!(writer.next().write_bool(true));
///     return Ok(());
/// }).unwrap();
/// assert_eq!(der, vec![2, 1, 10, 1, 1, 255]);
/// ```
///
/// # Errors
///
/// This function just carries errors generated by `callback`.
///
pub fn construct_der_seq<F>(callback: F) -> io::Result<Vec<u8>>
        where F: FnOnce(&mut DERWriterSeq) -> io::Result<()> {
    let mut buf = Vec::new();
    {
        let mut writer = DERWriterSeq {
            buf: &mut buf,
        };
        try!(callback(&mut writer));
    }
    return Ok(buf);
}

/// A writer object that accepts an ASN.1 value.
///
/// The two main sources of `DERWriterSeq` are:
///
/// - The [`construct_der`][construct_der] function, the starting point of
///   DER serialization.
/// - The `next` method of [`DERWriterSeq`][derwriterseq].
///
/// [construct_der]: fn.construct_der.html
/// [derwriterseq]: struct.DERWriterSeq.html
///
/// # Examples
///
/// ```
/// use yasna;
/// let der = yasna::construct_der(|writer| {
///     writer.write_i64(10)
/// }).unwrap();
/// assert_eq!(der, vec![2, 1, 10]);
/// ```
#[derive(Debug)]
pub struct DERWriter<'a> {
    buf: &'a mut Vec<u8>,
}

impl<'a> DERWriter<'a> {
    /// Writes BER identifier (tag + primitive/constructed) octets.
    fn write_identifier(&mut self, tag: Tag, pc: PC) -> io::Result<()> {
        let classid = tag.tag_class as u8;
        let pcid = pc as u8;
        if tag.tag_number < 31 {
            self.buf.push(
                (classid << 6) | (pcid << 5) | (tag.tag_number as u8));
            return Ok(());
        }
        self.buf.push((classid << 6) | (pcid << 5) | 31);
        let mut shiftnum = 63; // ceil(64 / 7) * 7 - 7
        while (tag.tag_number >> shiftnum) == 0 {
            shiftnum -= 7;
        }
        while shiftnum > 0 {
            self.buf.push(128 | (((tag.tag_number >> shiftnum) & 127) as u8));
            shiftnum -= 7;
        }
        self.buf.push((tag.tag_number & 127) as u8);
        return Ok(());
    }

    /// Writes BER length octets.
    fn write_length(&mut self, length: usize) -> io::Result<()> {
        let length = length as u64;
        if length < 128 {
            self.buf.push(length as u8);
            return Ok(());
        }
        let mut shiftnum = 56; // ceil(64 / 8) * 8 - 8
        while (length >> shiftnum) == 0 {
            shiftnum -= 8;
        }
        self.buf.push(128 | ((shiftnum / 8 + 1) as u8));
        loop {
            self.buf.push((length >> shiftnum) as u8);
            if shiftnum == 0 {
                break;
            }
            shiftnum -= 8;
        }
        return Ok(());
    }

    /// Deals with unknown length procedures.
    /// This function first marks the current position and
    /// allocates 3 bytes. Then it calls back `callback`.
    /// It then calculates the length and moves the written data
    /// to the actual position. Finally, it writes the length.
    fn with_length<T, F>(&mut self, callback: F) -> io::Result<T>
        where F: FnOnce(&mut Self) -> io::Result<T> {
        let expected_length_length = 3;
        for _ in 0..3 {
            self.buf.push(255);
        }
        let start_pos = self.buf.len();
        let result = try!(callback(self));
        let length = (self.buf.len() - start_pos) as u64;
        let length_length;
        let mut shiftnum = 56; // ceil(64 / 8) * 8 - 8
        if length < 128 {
            length_length = 1;
        } else {
            while (length >> shiftnum) == 0 {
                shiftnum -= 8;
            }
            length_length = shiftnum / 8 + 2;
        }
        let new_start_pos;
        if length_length < expected_length_length {
            let diff = expected_length_length - length_length;
            new_start_pos = start_pos - diff;
            self.buf.drain(new_start_pos .. start_pos);
        } else if length_length > expected_length_length {
            let diff = length_length - expected_length_length;
            new_start_pos = start_pos + diff;
            for _ in 0..diff { self.buf.insert(start_pos, 0); }
        } else {
            new_start_pos = start_pos;
        }
        let mut idx = new_start_pos - length_length;
        if length < 128 {
            self.buf[idx] = length as u8;
        } else {
            self.buf[idx] = 128 | ((shiftnum / 8 + 1) as u8);
            idx += 1;
            loop {
                self.buf[idx] = (length >> shiftnum) as u8;
                idx += 1;
                if shiftnum == 0 {
                    break;
                }
                shiftnum -= 8;
            }
        }
        return Ok(result);
    }

    /// Writes `bool` as an ASN.1 BOOLEAN value.
    ///
    /// # Examples
    ///
    /// ```
    /// use yasna;
    /// let der = yasna::construct_der(|writer| {
    ///     writer.write_bool(true)
    /// }).unwrap();
    /// assert_eq!(der, vec![1, 1, 255]);
    /// ```
    pub fn write_bool(mut self, val: bool) -> io::Result<()> {
        try!(self.write_identifier(TAG_BOOLEAN, PC::Primitive));
        try!(self.write_length(1));
        self.buf.push(if val { 255 } else { 0 });
        return Ok(());
    }

    /// Writes `i64` as an ASN.1 INTEGER value.
    ///
    /// # Examples
    ///
    /// ```
    /// use yasna;
    /// let der = yasna::construct_der(|writer| {
    ///     writer.write_i64(1234567890)
    /// }).unwrap();
    /// assert_eq!(der, vec![2, 4, 73, 150, 2, 210]);
    /// ```
    pub fn write_i64(mut self, val: i64) -> io::Result<()> {
        let mut shiftnum = 56;
        while shiftnum > 0 &&
                (val >> (shiftnum-1) == 0 || val >> (shiftnum-1) == -1) {
            shiftnum -= 8;
        }
        try!(self.write_identifier(TAG_INTEGER, PC::Primitive));
        try!(self.write_length(shiftnum / 8 + 1));
        loop {
            self.buf.push((val >> shiftnum) as u8);
            if shiftnum == 0 {
                break;
            }
            shiftnum -= 8;
        }
        return Ok(());
    }

    /// Writes `u64` as an ASN.1 INTEGER value.
    pub fn write_u64(mut self, val: u64) -> io::Result<()> {
        let mut shiftnum = 64;
        while shiftnum > 0 && val >> (shiftnum-1) == 0 {
            shiftnum -= 8;
        }
        try!(self.write_identifier(TAG_INTEGER, PC::Primitive));
        try!(self.write_length(shiftnum / 8 + 1));
        if shiftnum == 64 {
            self.buf.push(0);
            shiftnum -= 8;
        }
        loop {
            self.buf.push((val >> shiftnum) as u8);
            if shiftnum == 0 {
                break;
            }
            shiftnum -= 8;
        }
        return Ok(());
    }

    /// Writes `i32` as an ASN.1 INTEGER value.
    pub fn write_i32(self, val: i32) -> io::Result<()> {
        self.write_i64(val as i64)
    }

    /// Writes `u32` as an ASN.1 INTEGER value.
    pub fn write_u32(self, val: u32) -> io::Result<()> {
        self.write_i64(val as i64)
    }

    /// Writes `i16` as an ASN.1 INTEGER value.
    pub fn write_i16(self, val: i16) -> io::Result<()> {
        self.write_i64(val as i64)
    }

    /// Writes `u16` as an ASN.1 INTEGER value.
    pub fn write_u16(self, val: u16) -> io::Result<()> {
        self.write_i64(val as i64)
    }

    /// Writes `i8` as an ASN.1 INTEGER value.
    pub fn write_i8(self, val: i8) -> io::Result<()> {
        self.write_i64(val as i64)
    }

    /// Writes `u8` as an ASN.1 INTEGER value.
    pub fn write_u8(self, val: u8) -> io::Result<()> {
        self.write_i64(val as i64)
    }

    #[cfg(feature = "bigint")]
    /// Writes `BigInt` as an ASN.1 INTEGER value.
    ///
    /// # Examples
    ///
    /// ```
    /// # extern crate num;
    /// # extern crate yasna;
    /// # fn main() {
    /// use yasna;
    /// use num::bigint::BigInt;
    /// let der = yasna::construct_der(|writer| {
    ///     writer.write_bigint(
    ///         &BigInt::parse_bytes(b"1234567890", 10).unwrap())
    /// }).unwrap();
    /// assert_eq!(der, vec![2, 4, 73, 150, 2, 210]);
    /// # }
    /// ```
    pub fn write_bigint(mut self, val: &BigInt) -> io::Result<()> {
        use num::bigint::Sign;
        try!(self.write_identifier(TAG_INTEGER, PC::Primitive));
        let (sign, mut bytes) = val.to_bytes_le();
        match sign {
            Sign::NoSign => {
                try!(self.write_length(1));
                self.buf.push(0);
                return Ok(());
            },
            Sign::Plus => {
                let byteslen = bytes.len();
                debug_assert!(bytes[byteslen-1] != 0);
                if bytes[byteslen-1] >= 128 {
                    try!(self.write_length(byteslen+1));
                    self.buf.push(0);
                } else {
                    try!(self.write_length(byteslen));
                }
                bytes.reverse();
                self.buf.extend_from_slice(&bytes);
                return Ok(());
            },
            Sign::Minus => {
                let byteslen = bytes.len();
                debug_assert!(bytes[byteslen-1] != 0);
                let mut carry : usize = 1;
                for b in bytes.iter_mut() {
                    let bval = 255 - (*b as usize);
                    *b = (bval + carry) as u8;
                    carry = (bval + carry) >> 8;
                }
                if bytes[byteslen-1] < 128 {
                    try!(self.write_length(byteslen+1));
                    self.buf.push(255);
                } else {
                    try!(self.write_length(byteslen));
                }
                bytes.reverse();
                self.buf.extend_from_slice(&bytes);
                return Ok(());
            }
        };
    }

    #[cfg(feature = "bigint")]
    /// Writes `BigUint` as an ASN.1 INTEGER value.
    ///
    /// # Examples
    ///
    /// ```
    /// # extern crate num;
    /// # extern crate yasna;
    /// # fn main() {
    /// use yasna;
    /// use num::bigint::BigUint;
    /// let der = yasna::construct_der(|writer| {
    ///     writer.write_biguint(
    ///         &BigUint::parse_bytes(b"1234567890", 10).unwrap())
    /// }).unwrap();
    /// assert_eq!(der, vec![2, 4, 73, 150, 2, 210]);
    /// # }
    /// ```
    pub fn write_biguint(mut self, val: &BigUint) -> io::Result<()> {
        try!(self.write_identifier(TAG_INTEGER, PC::Primitive));
        let mut bytes = val.to_bytes_le();
        if &bytes == &[0] {
            try!(self.write_length(1));
            self.buf.push(0);
            return Ok(());
        }
        let byteslen = bytes.len();
        debug_assert!(bytes[byteslen-1] != 0);
        if bytes[byteslen-1] >= 128 {
            try!(self.write_length(byteslen+1));
            self.buf.push(0);
        } else {
            try!(self.write_length(byteslen));
        }
        bytes.reverse();
        self.buf.extend_from_slice(&bytes);
        return Ok(());
    }

    /// Writes `&[u8]` as an ASN.1 OCTETSTRING value.
    ///
    /// # Examples
    ///
    /// ```
    /// use yasna;
    /// let der = yasna::construct_der(|writer| {
    ///     writer.write_bytes(b"Hello!")
    /// }).unwrap();
    /// assert_eq!(der, vec![4, 6, 72, 101, 108, 108, 111, 33]);
    /// ```
    pub fn write_bytes(mut self, bytes: &[u8]) -> io::Result<()> {
        try!(self.write_identifier(TAG_OCTETSTRING, PC::Primitive));
        try!(self.write_length(bytes.len()));
        self.buf.extend_from_slice(bytes);
        return Ok(());
    }

    /// Writes the ASN.1 NULL value.
    ///
    /// # Examples
    ///
    /// ```
    /// use yasna;
    /// let der = yasna::construct_der(|writer| {
    ///     writer.write_null()
    /// }).unwrap();
    /// assert_eq!(der, vec![5, 0]);
    /// ```
    pub fn write_null(mut self) -> io::Result<()> {
        try!(self.write_identifier(TAG_NULL, PC::Primitive));
        try!(self.write_length(0));
        return Ok(());
    }

    /// Writes ASN.1 SEQUENCE.
    ///
    /// This function uses the loan pattern: `callback` is called back with
    /// a [`DERWriterSeq`][derwriterseq], to which the contents of the
    /// SEQUENCE is written.
    ///
    /// [derwriterseq]: struct.DERWriterSeq.html
    ///
    /// # Examples
    ///
    /// ```
    /// use yasna;
    /// let der = yasna::construct_der(|writer| {
    ///     writer.write_sequence(|writer| {
    ///         try!(writer.next().write_i64(10));
    ///         try!(writer.next().write_bool(true));
    ///         return Ok(());
    ///     })
    /// }).unwrap();
    /// assert_eq!(der, vec![48, 6, 2, 1, 10, 1, 1, 255]);
    /// ```
    pub fn write_sequence<T, F>(mut self, callback: F) -> io::Result<T>
        where F: FnOnce(&mut DERWriterSeq) -> io::Result<T> {
        try!(self.write_identifier(TAG_SEQUENCE, PC::Constructed));
        return self.with_length(|writer| {
            callback(&mut DERWriterSeq {
                buf: writer.buf,
            })
        });
    }


    /// Writes ASN.1 SET.
    ///
    /// This function uses the loan pattern: `callback` is called back with
    /// a [`DERWriterSet`][derwriterset], to which the contents of the
    /// SET is written.
    ///
    /// [derwriterset]: struct.DERWriterSet.html
    ///
    /// # Examples
    ///
    /// ```
    /// use yasna;
    /// let der = yasna::construct_der(|writer| {
    ///     writer.write_set(|writer| {
    ///         try!(writer.next().write_i64(10));
    ///         try!(writer.next().write_bool(true));
    ///         return Ok(());
    ///     })
    /// }).unwrap();
    /// assert_eq!(der, vec![49, 6, 1, 1, 255, 2, 1, 10]);
    /// ```
    pub fn write_set<T, F>(mut self, callback: F) -> io::Result<T>
        where F: FnOnce(&mut DERWriterSet) -> io::Result<T> {
        let mut bufs = Vec::new();
        let result = try!(callback(&mut DERWriterSet {
            bufs: &mut bufs,
        }));
        for buf in bufs.iter() {
            assert!(buf.len() > 0, "Empty output in write_set()");
        }
        bufs.sort_by(|buf0, buf1| {
            let buf00 = buf0[0] & 223;
            let buf10 = buf1[0] & 223;
            if buf00 != buf10 || (buf0[0] & 31) != 31 {
                return buf00.cmp(&buf10);
            }
            let len0 = buf0[1..].iter().position(|x| x & 128 == 0).unwrap();
            let len1 = buf1[1..].iter().position(|x| x & 128 == 0).unwrap();
            if len0 != len1 {
                return len0.cmp(&len1);
            }
            return buf0[1..].cmp(&buf1[1..]);
        });
        // let bufs_len = bufs.iter().map(|buf| buf.len()).sum();
        let bufs_len = bufs.iter().map(|buf| buf.len()).fold(0, |x, y| x + y);
        try!(self.write_identifier(TAG_SET, PC::Constructed));
        try!(self.write_length(bufs_len));
        for buf in bufs.iter() {
            self.buf.extend_from_slice(buf);
        }
        return Ok(result);
    }
}

/// A writer object that accepts ASN.1 values.
///
/// The main source of this object is the `write_sequence` method from
/// [`DERWriter`][derwriter].
///
/// [derwriter]: struct.DERWriter.html
///
/// # Examples
///
/// ```
/// use yasna;
/// let der = yasna::construct_der(|writer| {
///     writer.write_sequence(|writer : &mut yasna::DERWriterSeq| {
///         try!(writer.next().write_i64(10));
///         try!(writer.next().write_bool(true));
///         return Ok(());
///     })
/// }).unwrap();
/// assert_eq!(der, vec![48, 6, 2, 1, 10, 1, 1, 255]);
/// ```
#[derive(Debug)]
pub struct DERWriterSeq<'a> {
    buf: &'a mut Vec<u8>,
}

impl<'a> DERWriterSeq<'a> {
    /// Generates a new [`DERWriter`][derwriter].
    ///
    /// [derwriter]: struct.DERWriter.html
    pub fn next<'b>(&'b mut self) -> DERWriter<'b> {
        return DERWriter {
            buf: self.buf,
        };
    }
}

/// A writer object that accepts ASN.1 values.
///
/// The main source of this object is the `write_set` method from
/// [`DERWriter`][derwriter].
///
/// [derwriter]: struct.DERWriter.html
///
/// # Examples
///
/// ```
/// use yasna;
/// let der = yasna::construct_der(|writer| {
///     writer.write_set(|writer : &mut yasna::DERWriterSet| {
///         try!(writer.next().write_i64(10));
///         try!(writer.next().write_bool(true));
///         return Ok(());
///     })
/// }).unwrap();
/// assert_eq!(der, vec![49, 6, 1, 1, 255, 2, 1, 10]);
/// ```
#[derive(Debug)]
pub struct DERWriterSet<'a> {
    bufs: &'a mut Vec<Vec<u8>>,
}

impl<'a> DERWriterSet<'a> {
    /// Generates a new [`DERWriter`][derwriter].
    ///
    /// [derwriter]: struct.DERWriter.html
    pub fn next<'b>(&'b mut self) -> DERWriter<'b> {
        self.bufs.push(Vec::new());
        return DERWriter {
            buf: self.bufs.last_mut().unwrap(),
        };
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
enum PC {
    Primitive = 0, Constructed = 1,
}

#[cfg(test)]
mod tests;
